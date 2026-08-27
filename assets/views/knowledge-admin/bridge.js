// Greentic view bridge, protocol v1.
//
// The page runs in an iframe with an opaque origin, so it can reach nothing on
// its own: no host cookies, no localStorage, no parent DOM, and its own
// fetch() would send `Origin: null`. Everything goes through the host, which
// holds the credentials and applies the caller's own permissions to each
// request. The bridge asks for results; it never receives keys — though the
// results it gets back (tool output, callApi bodies) can still be
// confidential to the viewer, so treat this channel as sensitive even though
// no credential crosses it.
(function (global) {
  const PROTOCOL = 1;
  // How long to wait for a reply before giving up and freeing the slot. A
  // disabled extension, a host that navigated away, or a message `type` the
  // host doesn't implement all look identical from here: silence. Without a
  // timeout the Promise — and its `pending`/`ready` entry — would live for
  // the rest of the page's life.
  const CALL_TIMEOUT_MS = 10000;
  const INIT_TIMEOUT_MS = 10000;

  const pending = new Map();
  let nextId = 0;

  let readyResolve;
  let readyReject;
  const ready = new Promise((resolve, reject) => {
    readyResolve = resolve;
    readyReject = reject;
  });
  const readyTimer = global.setTimeout(() => {
    readyReject(
      new Error(
        "no init message received from host within " + INIT_TIMEOUT_MS + "ms"
      )
    );
  }, INIT_TIMEOUT_MS);

  function send(message) {
    // targetOrigin cannot be pinned: an opaque origin has no name to pin to.
    // The host posts with "*" for the same reason.
    //
    // `message` is spread first so the protocol version always wins: no
    // current caller passes a `v` key, but this file is copied verbatim by
    // authors, and a payload that could clobber the protocol tag is a trap
    // worth closing even before something exploits it.
    global.parent.postMessage(
      Object.assign({}, message, { v: PROTOCOL }),
      "*"
    );
  }

  function call(type, payload) {
    const id = "c" + ++nextId;
    return new Promise((resolve, reject) => {
      const timer = global.setTimeout(() => {
        pending.delete(id);
        reject(
          new Error(
            "bridge call " +
              id +
              " (" +
              type +
              ") timed out after " +
              CALL_TIMEOUT_MS +
              "ms: no response from host"
          )
        );
      }, CALL_TIMEOUT_MS);
      pending.set(id, {
        resolve: (data) => {
          global.clearTimeout(timer);
          resolve(data);
        },
        reject: (err) => {
          global.clearTimeout(timer);
          reject(err);
        },
      });
      // Same ordering rule as `send`: the caller's own payload is spread
      // first, so it can never clobber the correlation id or message type
      // the reply is matched against.
      send(Object.assign({}, payload, { id: id, type: type }));
    });
  }

  global.addEventListener("message", (event) => {
    // event.origin cannot be used to identify the sender: this page has an
    // opaque origin (sandboxed iframe, no allow-same-origin), so the real
    // host's messages arrive with event.origin === "null" — and a forged
    // message from any other window would look identical by that measure.
    // The one thing that does identify the sender is *which window object*
    // posted the message: only the parent frame is a legitimate bridge peer.
    if (event.source !== global.parent) return;

    const msg = event.data;
    if (!msg || msg.v !== PROTOCOL) return;

    if (msg.type === "init") {
      global.clearTimeout(readyTimer);
      readyResolve(msg);
      return;
    }
    if (msg.type === "result") {
      const slot = pending.get(msg.id);
      if (!slot) return;
      pending.delete(msg.id);
      if (msg.ok) slot.resolve(msg.data);
      else slot.reject(new Error(msg.error ? msg.error.message : "bridge call failed"));
    }
  });

  global.greentic = {
    protocol: PROTOCOL,
    /** Resolves with the host's `init` message: locale, theme, surface, context. */
    ready: ready,
    /** Invoke one of this extension's own tools, as listed in `views[].tools`. */
    invokeTool: (name, args) => call("invokeTool", { name: name, args: args || {} }),
    /** Call a platform endpoint listed in `permissions.ui.platformApi`. */
    callApi: (method, path, body) => call("callApi", { method: method, path: path, body: body }),
    /** Fetch a host listed in `permissions.ui.fetchHosts`, proxied server-side. */
    fetch: (url, options) => call("fetch", { url: url, options: options || {} }),
    /** Tell the host how tall the page is, so it can size the frame. */
    resize: (height) => send({ type: "resize", height: height }),
    /** Ask the host to navigate its own router. */
    navigate: (to) => send({ type: "navigate", to: to }),
    toast: (level, message) => send({ type: "toast", level: level, message: message }),
  };
})(window);

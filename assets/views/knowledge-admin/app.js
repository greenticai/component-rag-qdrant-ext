// Knowledge-base view for greentic.rag-qdrant.
//
// Runs in a sandboxed iframe with an opaque origin: no host cookies, no
// localStorage, no parent DOM, and its own fetch() would be useless. Every
// piece of data on this page arrives through `window.greentic.invokeTool`,
// which the host executes with the viewer's own permissions. See bridge.js.
//
// Two rules this file follows without exception:
//
//   1. Never `innerHTML` a value that came back from a tool. Document text,
//      doc_ids and metadata are attacker-influenced (anyone who can ingest
//      can choose them), and script injected into this frame would inherit
//      the bridge — i.e. the right to call every tool in `views[].tools`.
//      Everything user-derived goes in through `textContent`.
//   2. Never `window.confirm` / `window.alert`. A native modal blocks this
//      frame's event loop, and with it the `message` handler the bridge
//      needs to receive replies — every in-flight call would time out at ten
//      seconds. Confirmation is in-page.
(function () {
  "use strict";

  // ---------------------------------------------------------------- config

  // `rag_list`'s `limit` counts *chunks scanned*, not documents returned, so a
  // small limit shows very few documents. 200 chunks is roughly 20-40 typical
  // documents per page.
  var LIST_LIMIT = 200;
  var PREVIEW_CHARS = 4000;
  var MAX_DOC_ID = 200;

  // Every bridge call — `rag_ingest` included — is abandoned by bridge.js
  // after 10s. `rag_ingest` embeds every chunk in one request before it
  // writes, so a long document can exceed that. These thresholds only drive
  // warnings; the user is never blocked, because the host may well be fast
  // enough and only the operator knows their embedding endpoint.
  var CHARS_SOFT_WARN = 150000;
  var CHARS_HARD_WARN = 400000;

  // Hard ceiling on what this page will even read. Generous for prose in the
  // formats below, and low enough that a mis-picked disk image is refused
  // instead of buffered.
  var MAX_FILE_BYTES = 25 * 1024 * 1024;

  var TEXT_EXTENSIONS = [
    "txt",
    "text",
    "md",
    "markdown",
    "mdown",
    "mkd",
    "csv",
    "tsv",
  ];

  // ------------------------------------------------------------------ DOM

  function el(id) {
    return document.getElementById(id);
  }

  var dom = {
    app: el("app"),
    status: el("status"),
    workspace: el("workspace"),
    tabDocuments: el("tab-documents"),
    tabSearch: el("tab-search"),
    panelDocuments: el("panel-documents"),
    panelSearch: el("panel-search"),
    drop: el("drop"),
    file: el("file"),
    extract: el("extract"),
    extractLabel: el("extract-label"),
    extractBar: el("extract-bar"),
    staged: el("staged"),
    docId: el("doc-id"),
    stagedWarnings: el("staged-warnings"),
    stagedSummary: el("staged-summary"),
    stagedPreview: el("staged-preview"),
    ingest: el("ingest"),
    ingestCancel: el("ingest-cancel"),
    refresh: el("refresh"),
    docs: el("docs"),
    prevPage: el("prev-page"),
    nextPage: el("next-page"),
    searchForm: el("search-form"),
    searchQ: el("search-q"),
    searchK: el("search-k"),
    searchGo: el("search-go"),
    hits: el("hits"),
    toast: el("toast"),
    bar: el("bar"),
    replaceNotice: el("replace-notice"),
  };

  // ---------------------------------------------------------------- state

  var state = {
    connected: false,
    locale: "en",
    // The document staged for ingest: { text, warnings, filename, sourceType }.
    staged: null,
    busy: false,
    documents: [],
    // `next_page_offset` is documented as an opaque cursor, so it is stored
    // and replayed verbatim and never inspected.
    nextOffset: null,
    // Offsets of the pages already visited, so "Previous page" can replay
    // one. Qdrant's scroll cursor only moves forward.
    history: [],
    currentOffset: null,
    confirmingDocId: null,
    listError: null,
    listLoading: false,
    // Monotonic ticket for list calls. Only the newest call may write state,
    // so an older, slower rag_list cannot land last and overwrite a newer
    // page — or clobber its error with a stale success.
    listSeq: 0,
  };

  // --------------------------------------------------------------- helpers

  function setStatus(text, kind) {
    dom.status.textContent = text;
    dom.status.className = "status" + (kind ? " status--" + kind : "");
  }

  var messageTimer = null;

  /** In-page replacement for alert(): never blocks the bridge. */
  function showMessage(kind, text) {
    dom.toast.textContent = text;
    dom.toast.className = "pagemsg" + (kind ? " pagemsg--" + kind : "");
    dom.toast.hidden = false;
    if (messageTimer !== null) {
      window.clearTimeout(messageTimer);
    }
    // Errors stay put; successes clear themselves.
    if (kind !== "error") {
      messageTimer = window.setTimeout(function () {
        dom.toast.hidden = true;
        scheduleResize();
      }, 6000);
    }
    scheduleResize();
  }

  function clearMessage() {
    dom.toast.hidden = true;
    dom.toast.textContent = "";
    scheduleResize();
  }

  /**
   * `busy` gates every action, and `renderDocuments` paints each row's Delete
   * from it. Flipping the flag without re-rendering therefore strands those
   * buttons disabled until the next list call — so the flag is only ever set
   * through here, never assigned directly.
   */
  function setBusy(value) {
    state.busy = value;
    renderDocuments();
  }

  /** Tell the user why their click did nothing, rather than dropping it. */
  function rejectWhileBusy() {
    showMessage("info", "Still finishing the last action — one moment.");
  }

  var resizeTimer = null;

  /** Tell the host how tall we are, coalescing bursts of DOM changes. */
  function scheduleResize() {
    if (resizeTimer !== null) {
      window.clearTimeout(resizeTimer);
    }
    resizeTimer = window.setTimeout(function () {
      resizeTimer = null;
      if (state.connected) {
        window.greentic.resize(document.body.scrollHeight);
      }
    }, 50);
  }

  /** Hand the event loop back so a long job cannot freeze the tab. */
  function yieldToUi() {
    return new Promise(function (resolve) {
      window.setTimeout(resolve, 0);
    });
  }

  function clear(node) {
    while (node.firstChild) {
      node.removeChild(node.firstChild);
    }
  }

  function make(tag, className, text) {
    var node = document.createElement(tag);
    if (className) {
      node.className = className;
    }
    if (text !== undefined && text !== null) {
      node.textContent = String(text);
    }
    return node;
  }

  function errorText(err) {
    if (err && typeof err.message === "string" && err.message) {
      return err.message;
    }
    return String(err);
  }

  /**
   * bridge.js gives up on a call after 10s. That reads as a failure here but
   * often is not one server-side: `rag_ingest` deletes and re-upserts
   * regardless of whether anyone is still listening.
   */
  function isTimeout(err) {
    return /timed out/i.test(errorText(err));
  }

  function extensionOf(name) {
    var dot = name.lastIndexOf(".");
    if (dot < 0 || dot === name.length - 1) {
      return "";
    }
    return name.slice(dot + 1).toLowerCase();
  }

  function formatBytes(n) {
    if (n < 1024) {
      return n + " B";
    }
    if (n < 1024 * 1024) {
      return (n / 1024).toFixed(1) + " KB";
    }
    return (n / (1024 * 1024)).toFixed(1) + " MB";
  }

  /**
   * A doc_id is the handle the user later deletes or replaces by, so it is
   * derived from the filename but kept readable and editable rather than
   * hashed.
   */
  function suggestDocId(filename) {
    var dot = filename.lastIndexOf(".");
    var stem = dot > 0 ? filename.slice(0, dot) : filename;
    var cleaned = stem
      .replace(/[\u0000-\u001f\u007f]/g, "")
      .replace(/\s+/g, " ")
      .trim();
    if (!cleaned) {
      cleaned = "document";
    }
    return cleaned.slice(0, MAX_DOC_ID);
  }

  // --------------------------------------------------- text extraction

  /**
   * Decode a text/Markdown file. UTF-8 first, strictly — a strict decode that
   * throws is the only reliable signal that the bytes are not UTF-8. Only
   * then fall back to windows-1252, which decodes any byte sequence and so
   * can never fail, only be wrong.
   */
  function decodeTextBytes(buffer, warnings) {
    var bytes = new Uint8Array(buffer);
    // Native indexOf, not a JS loop: this runs over every byte of the file.
    if (bytes.indexOf(0) !== -1) {
      throw new Error(
        "This file contains binary data, not text. Save it as plain text, " +
          "Markdown or PDF and try again."
      );
    }
    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    } catch (err) {
      warnings.push(
        "The file is not valid UTF-8; it was read as Windows-1252 instead. " +
          "Check the preview for wrong accented characters."
      );
      return new TextDecoder("windows-1252").decode(bytes);
    }
  }

  function normaliseText(text) {
    return text
      .replace(/^\uFEFF/, "")
      .replace(/\r\n?/g, "\n")
      .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "");
  }

  function setProgress(fraction, label) {
    dom.extract.hidden = false;
    dom.extractLabel.textContent = label;
    var safe = Number.isFinite(fraction) ? fraction : 0;
    var pct = Math.max(0, Math.min(1, safe)) * 100;
    dom.extractBar.style.width = pct.toFixed(1) + "%";
    dom.bar.setAttribute("aria-valuenow", String(Math.round(pct)));
  }

  function hideProgress() {
    dom.extract.hidden = true;
    dom.extractBar.style.width = "0%";
  }

  /**
   * Turn a picked file into text, entirely in this browser. Nothing leaves
   * the page until the user presses "Add to knowledge base".
   *
   * Rejects loudly on anything it cannot read properly. Silently ingesting a
   * mangled decode would poison the knowledge base in a way that is very hard
   * to notice afterwards — the chunks look fine in a listing and only ever
   * surface as bad search results.
   */
  async function extractFile(file) {
    var warnings = [];
    var ext = extensionOf(file.name);
    var mime = (file.type || "").toLowerCase();
    var isPdf = ext === "pdf" || mime === "application/pdf";
    var isText =
      TEXT_EXTENSIONS.indexOf(ext) >= 0 ||
      mime === "text/plain" ||
      mime === "text/markdown" ||
      (mime.indexOf("text/") === 0 && ext === "");

    if (!isPdf && !isText) {
      throw new Error(
        'Cannot read "' +
          file.name +
          '". This page reads plain text (.txt, .csv), Markdown (.md) and ' +
          "PDF (.pdf). For anything else — Word, Google Docs, HTML, slides — " +
          "export or copy it to one of those first."
      );
    }

    // Checked before the read, not after: `file.arrayBuffer()` buffers the
    // whole file in this frame, and the extracted text is then structure-
    // cloned across postMessage to the host, so an unbounded file can take
    // the host's tab down with this one. The character-count warnings later
    // on only fire once that damage is already done.
    if (file.size > MAX_FILE_BYTES) {
      throw new Error(
        '"' +
          file.name +
          '" is ' +
          formatBytes(file.size) +
          ", which is too large to read in the browser (the limit is " +
          formatBytes(MAX_FILE_BYTES) +
          "). Split it into smaller documents and add them one at a time."
      );
    }

    setProgress(0.05, "Reading " + file.name + "…");
    await yieldToUi();
    var buffer = await file.arrayBuffer();

    if (isPdf) {
      if (!window.ragPdf || typeof window.ragPdf.extractPdfText !== "function") {
        throw new Error(
          "PDF reading is unavailable in this build. Convert the PDF to text " +
            "or Markdown and upload that instead."
        );
      }
      setProgress(0.15, "Reading text out of " + file.name + "…");
      await yieldToUi();
      var pdf = await window.ragPdf.extractPdfText(buffer, function (
        fraction,
        label
      ) {
        // Map the extractor's own 0..1 onto the back four fifths of the bar.
        setProgress(0.15 + fraction * 0.8, label || "Reading the PDF…");
      });
      Array.prototype.push.apply(warnings, pdf.warnings || []);
      setProgress(1, "Done.");
      return {
        text: normaliseText(pdf.text),
        warnings: warnings,
        sourceType: "pdf",
        pages: pdf.pages,
      };
    }

    setProgress(0.6, "Decoding " + file.name + "…");
    await yieldToUi();
    var text = normaliseText(decodeTextBytes(buffer, warnings));
    setProgress(1, "Done.");
    return {
      text: text,
      warnings: warnings,
      sourceType:
        ext === "md" || ext === "markdown" || ext === "mdown" || ext === "mkd"
          ? "markdown"
          : "text",
      pages: null,
    };
  }

  // ------------------------------------------------------------- staging

  function renderStaged() {
    var staged = state.staged;
    if (!staged) {
      dom.staged.hidden = true;
      scheduleResize();
      return;
    }

    dom.staged.hidden = false;

    var bits = [
      formatBytes(staged.bytes),
      staged.text.length.toLocaleString(state.locale) + " characters",
    ];
    if (staged.pages) {
      bits.push(staged.pages + (staged.pages === 1 ? " page" : " pages"));
    }
    dom.stagedSummary.textContent = staged.filename + " — " + bits.join(", ");

    var preview = staged.text.slice(0, PREVIEW_CHARS);
    dom.stagedPreview.textContent =
      staged.text.length > PREVIEW_CHARS
        ? preview + "\n\n… (preview truncated; the whole document is sent)"
        : preview;

    var warnings = staged.warnings.slice();
    if (staged.text.length >= CHARS_HARD_WARN) {
      warnings.push(
        "This document is very long. Adding it means embedding every chunk in " +
          "one request, and the host stops waiting after 10 seconds. It will " +
          "probably report a timeout even if it succeeds — refresh the list to " +
          "check. Splitting the document into a few smaller files is more reliable."
      );
    } else if (staged.text.length >= CHARS_SOFT_WARN) {
      warnings.push(
        "This is a long document. Adding it may take a while, and the host " +
          "stops waiting after 10 seconds; if it reports a timeout, refresh the " +
          "list before trying again."
      );
    }

    clear(dom.stagedWarnings);
    if (warnings.length === 0) {
      dom.stagedWarnings.hidden = true;
    } else {
      dom.stagedWarnings.hidden = false;
      dom.stagedWarnings.appendChild(
        make("strong", null, "Before you add this:")
      );
      var list = make("ul");
      warnings.forEach(function (warning) {
        list.appendChild(make("li", null, warning));
      });
      dom.stagedWarnings.appendChild(list);
    }

    renderReplaceNotice();
    scheduleResize();
  }

  /**
   * Warn, at the moment of naming, that this name is already taken and adding
   * will replace it. Only the current page of documents is known here, so
   * absence proves nothing — the notice is only ever shown on a positive
   * match, never as a "this is new" reassurance.
   */
  function renderReplaceNotice() {
    var docId = dom.docId.value.trim();
    var clash =
      docId.length > 0 &&
      state.documents.some(function (doc) {
        return doc.doc_id === docId;
      });
    dom.replaceNotice.hidden = !clash;
    dom.replaceNotice.textContent = clash
      ? "“" +
        docId +
        "” already exists. Adding this will replace it — the old version's " +
        "content is removed."
      : "";
  }

  function discardStaged() {
    state.staged = null;
    dom.file.value = "";
    dom.docId.value = "";
    hideProgress();
    renderStaged();
  }

  async function stageFile(file) {
    if (state.busy) {
      rejectWhileBusy();
      return;
    }
    clearMessage();
    setBusy(true);
    dom.ingest.disabled = true;
    try {
      var extracted = await extractFile(file);
      if (!extracted.text.trim()) {
        throw new Error(
          '"' + file.name + '" contains no text to add.'
        );
      }
      state.staged = {
        filename: file.name,
        bytes: file.size,
        text: extracted.text,
        warnings: extracted.warnings,
        sourceType: extracted.sourceType,
        pages: extracted.pages,
      };
      dom.docId.value = suggestDocId(file.name);
      renderStaged();
      hideProgress();
    } catch (err) {
      // Deliberately NOT discardStaged(): a document staged earlier is still
      // good, and throwing it away because the *next* file was unreadable
      // would make the user re-pick and re-extract it for nothing.
      hideProgress();
      showMessage("error", errorText(err));
    } finally {
      setBusy(false);
      dom.ingest.disabled = false;
      dom.file.value = "";
      scheduleResize();
    }
  }

  // -------------------------------------------------------------- ingest

  async function doIngest() {
    if (!state.staged) {
      return;
    }
    if (state.busy) {
      rejectWhileBusy();
      return;
    }
    var docId = dom.docId.value.trim();
    if (!docId) {
      showMessage("error", "Give the document a name before adding it.");
      dom.docId.focus();
      return;
    }
    if (docId.length > MAX_DOC_ID) {
      showMessage(
        "error",
        "That name is too long — keep it under " + MAX_DOC_ID + " characters."
      );
      dom.docId.focus();
      return;
    }

    clearMessage();
    setBusy(true);
    // Discard and the name field are locked too: discarding mid-flight would
    // throw away the extracted text, which is the only thing that makes a
    // failed or timed-out ingest retryable without re-reading the file.
    dom.ingest.disabled = true;
    dom.ingestCancel.disabled = true;
    dom.docId.disabled = true;
    dom.ingest.textContent = "Adding…";
    setProgress(0.5, "Adding “" + docId + "” to the knowledge base…");

    var wrote = false;
    try {
      // No `collection` argument, deliberately. The host stamps the calling
      // tenant's collection onto every call and the tool now *refuses* a
      // per-call override when it has done so, so sending one would turn every
      // ingest into an error. See README.md, "Collections and tenancy".
      var result = await window.greentic.invokeTool("rag_ingest", {
        doc_id: docId,
        text: state.staged.text,
        metadata: {
          filename: state.staged.filename,
          source_type: state.staged.sourceType,
          ingested_at: new Date().toISOString(),
        },
      });
      var chunks = result && typeof result.chunks === "number" ? result.chunks : null;
      showMessage(
        "ok",
        "Added “" +
          docId +
          "”" +
          (chunks === null ? "." : " as " + chunks + (chunks === 1 ? " chunk." : " chunks."))
      );
      wrote = true;
      discardStaged();
    } catch (err) {
      if (isTimeout(err)) {
        showMessage(
          "error",
          "The host stopped waiting after 10 seconds. “" +
            docId +
            "” may still have been added — refresh the list in a moment before " +
            "trying again. Re-adding it is safe: the same name always replaces, " +
            "never duplicates."
        );
      } else {
        showMessage("error", "Could not add the document: " + errorText(err));
      }
    } finally {
      dom.ingest.disabled = false;
      dom.ingestCancel.disabled = false;
      dom.docId.disabled = false;
      dom.ingest.textContent = "Add to knowledge base";
      hideProgress();
      setBusy(false);
      scheduleResize();
    }

    // Outside the try on purpose. Refreshing the list is a *separate* action
    // from the write: if rendering the new page threw, the catch above would
    // paint "Could not add the document" over a write that in fact succeeded.
    if (wrote) {
      await loadFirstPage();
    }
  }

  // ---------------------------------------------------------------- list

  function metadataSummary(metadata) {
    if (!metadata || typeof metadata !== "object" || Array.isArray(metadata)) {
      return "";
    }
    var parts = [];
    var shown = 0;
    var keys = Object.keys(metadata).filter(function (key) {
      return metadata[key] !== null && metadata[key] !== undefined;
    });
    keys.forEach(function (key) {
      if (shown >= 4) {
        return;
      }
      shown += 1;
      var value = metadata[key];
      var rendered =
        typeof value === "object" ? JSON.stringify(value) : String(value);
      if (rendered.length > 60) {
        rendered = rendered.slice(0, 57) + "…";
      }
      parts.push(key + ": " + rendered);
    });
    if (keys.length > shown) {
      parts.push("+" + (keys.length - shown) + " more");
    }
    return parts.join(" · ");
  }

  function renderDocuments() {
    // Re-rendering destroys the focused button. Remember which row owned
    // focus so a keyboard user is not dumped back to <body> on every render.
    var focusWas = document.activeElement;
    var refocus =
      focusWas && focusWas.dataset ? focusWas.dataset.focusKey || null : null;
    // Set before the early returns below, not after them: the loading branch
    // returns first, and that is exactly the branch this needs to cover.
    dom.refresh.disabled = state.listLoading;
    clear(dom.docs);

    if (state.listLoading) {
      dom.docs.appendChild(make("p", "empty", "Loading…"));
      dom.prevPage.hidden = true;
      dom.nextPage.hidden = true;
      scheduleResize();
      return;
    }

    if (state.listError) {
      dom.docs.appendChild(make("p", "error", state.listError));
      dom.prevPage.hidden = true;
      dom.nextPage.hidden = true;
      scheduleResize();
      return;
    }

    if (state.documents.length === 0) {
      dom.docs.appendChild(
        make(
          "p",
          "empty",
          state.history.length > 0
            ? "No documents on this page."
            : "No documents yet. Add one above."
        )
      );
    }

    state.documents.forEach(function (doc) {
      var confirming = state.confirmingDocId === doc.doc_id;
      var row = make("div", "doc" + (confirming ? " doc--confirming" : ""));

      var main = make("div", "doc__main");
      main.appendChild(make("div", "doc__id", doc.doc_id));

      var detail = doc.chunk_count + (doc.chunk_count === 1 ? " chunk" : " chunks");
      var meta = metadataSummary(doc.metadata);
      if (meta) {
        detail += " · " + meta;
      }
      main.appendChild(make("div", "doc__meta", detail));
      row.appendChild(main);

      if (confirming) {
        // In-page confirmation. window.confirm() would block this frame's
        // message handler and strand every bridge reply.
        var confirmBox = make("div", "confirm");
        confirmBox.appendChild(
          make(
            "span",
            "confirm__text",
            "Delete “" + doc.doc_id + "” and all of its chunks? This cannot be undone."
          )
        );
        var yes = make("button", "btn btn--danger", "Delete");
        yes.type = "button";
        // The ask button is disabled while busy; this one must be too, or it
        // looks live, does nothing, and leaves the confirmation open.
        yes.disabled = state.busy;
        yes.setAttribute("aria-label", "Confirm deleting " + doc.doc_id);
        yes.dataset.focusKey = "confirm:" + doc.doc_id;
        yes.addEventListener("click", function () {
          doDelete(doc.doc_id);
        });
        var no = make("button", "btn btn--ghost", "Cancel");
        no.type = "button";
        no.setAttribute("aria-label", "Keep " + doc.doc_id);
        no.dataset.focusKey = "cancel:" + doc.doc_id;
        no.addEventListener("click", function () {
          state.confirmingDocId = null;
          renderDocuments();
          focusByKey("ask:" + doc.doc_id);
        });
        confirmBox.appendChild(yes);
        confirmBox.appendChild(no);
        row.appendChild(confirmBox);
      } else {
        var ask = make("button", "btn btn--secondary", "Delete");
        ask.type = "button";
        ask.disabled = state.busy;
        // Without this every row's button is announced as bare "Delete".
        ask.setAttribute("aria-label", "Delete " + doc.doc_id);
        ask.dataset.focusKey = "ask:" + doc.doc_id;
        ask.addEventListener("click", function () {
          state.confirmingDocId = doc.doc_id;
          renderDocuments();
          focusByKey("confirm:" + doc.doc_id);
        });
        row.appendChild(ask);
      }

      dom.docs.appendChild(row);
    });

    dom.prevPage.hidden = state.history.length === 0;
    dom.nextPage.hidden =
      state.nextOffset === null || state.nextOffset === undefined;
    if (refocus) {
      focusByKey(refocus);
    }
    renderReplaceNotice();
    scheduleResize();
  }

  /**
   * Restore focus to a rebuilt button by its stable key, if it still exists.
   *
   * The key embeds a `doc_id`, which the person who ingested the document
   * chose. Building a `[data-focus-key="…"]` selector out of that would let a
   * crafted id break the selector syntax (or match the wrong row), so the
   * buttons are walked and compared as plain strings instead — there is no
   * selector to escape.
   */
  function focusByKey(key) {
    var buttons = dom.docs.getElementsByTagName("button");
    for (var i = 0; i < buttons.length; i += 1) {
      if (buttons[i].dataset.focusKey === key && !buttons[i].disabled) {
        buttons[i].focus();
        return;
      }
    }
  }

  /**
   * Fetch one scroll page. `offset` is replayed exactly as Qdrant handed it
   * back; it is documented as opaque and is never parsed here.
   */
  async function loadPage(offset) {
    var ticket = ++state.listSeq;
    state.listLoading = true;
    state.listError = null;
    state.confirmingDocId = null;
    renderDocuments();

    var args = { limit: LIST_LIMIT };
    if (offset !== null && offset !== undefined) {
      args.offset = offset;
    }

    try {
      var result = await window.greentic.invokeTool("rag_list", args);
      if (ticket !== state.listSeq) {
        return false;
      }
      state.documents =
        result && Array.isArray(result.documents)
          ? result.documents.filter(function (doc) {
              // A malformed row would throw during render and, worse, take a
              // surrounding action's error path with it.
              return doc && typeof doc.doc_id === "string";
            })
          : [];
      // The key is omitted entirely on the last page, so `undefined` and
      // `null` both mean "no more pages".
      var next = result ? result.next_page_offset : undefined;
      state.nextOffset = next === undefined ? null : next;
      state.currentOffset = offset === undefined ? null : offset;
      return true;
    } catch (err) {
      if (ticket !== state.listSeq) {
        return false;
      }
      state.documents = [];
      state.nextOffset = null;
      state.listError = isTimeout(err)
        ? "The host stopped waiting for the document list after 10 seconds. Try Refresh."
        : "Could not list documents: " + errorText(err);
      return false;
    } finally {
      if (ticket === state.listSeq) {
        state.listLoading = false;
        renderDocuments();
      }
    }
  }

  function loadFirstPage() {
    state.history = [];
    return loadPage(null);
  }

  async function goNextPage() {
    if (state.nextOffset === null || state.nextOffset === undefined) {
      return;
    }
    var from = state.currentOffset;
    var ok = await loadPage(state.nextOffset);
    // Push only on success, or a failed load leaves a phantom entry and
    // "Previous page" walks back to the page you are already on.
    if (ok) {
      state.history.push(from);
      renderDocuments();
    }
  }

  async function goPrevPage() {
    if (state.history.length === 0) {
      return;
    }
    var previous = state.history[state.history.length - 1];
    var ok = await loadPage(previous);
    // Pop only on success, so a failed load does not lose the way back.
    if (ok) {
      state.history.pop();
      renderDocuments();
    }
  }

  // -------------------------------------------------------------- delete

  async function doDelete(docId) {
    if (state.busy) {
      rejectWhileBusy();
      return;
    }
    clearMessage();
    state.confirmingDocId = null;
    setBusy(true);
    try {
      // `doc_id` only. The tool's schema is a oneOf over `ids` XOR `doc_id`,
      // so sending both — or sending `collection` alongside — is the easy way
      // to get an unhelpful validation error.
      await window.greentic.invokeTool("rag_delete", { doc_id: docId });
      showMessage("ok", "Deleted “" + docId + "”.");
      await loadPage(state.currentOffset);
    } catch (err) {
      showMessage("error", "Could not delete “" + docId + "”: " + errorText(err));
    } finally {
      setBusy(false);
      scheduleResize();
    }
  }

  // -------------------------------------------------------------- search

  function renderHits(hits, query) {
    clear(dom.hits);
    if (hits.length === 0) {
      dom.hits.appendChild(
        make("p", "empty", "Nothing matched “" + query + "”.")
      );
      scheduleResize();
      return;
    }

    hits.forEach(function (hit) {
      var payload =
        hit && hit.payload && typeof hit.payload === "object" ? hit.payload : {};
      var card = make("div", "hit");

      var head = make("div", "hit__head");
      var label = typeof payload.doc_id === "string" ? payload.doc_id : String(hit.id);
      if (typeof payload.chunk_index === "number") {
        label += " · chunk " + payload.chunk_index;
      }
      head.appendChild(make("span", "hit__label", label));

      var score = typeof hit.score === "number" ? hit.score.toFixed(4) : "—";
      head.appendChild(make("span", "hit__score", "score " + score));
      card.appendChild(head);

      var text =
        typeof payload.text === "string" && payload.text.trim()
          ? payload.text
          : "(this chunk stored no text)";
      card.appendChild(make("p", "hit__text", text));
      dom.hits.appendChild(card);
    });
    scheduleResize();
  }

  async function doSearch(event) {
    event.preventDefault();
    if (state.busy) {
      rejectWhileBusy();
      return;
    }
    var query = dom.searchQ.value.trim();
    if (!query) {
      showMessage("error", "Type something to search for.");
      return;
    }
    var topK = parseInt(dom.searchK.value, 10);
    if (!Number.isFinite(topK) || topK < 1) {
      topK = 5;
      dom.searchK.value = "5";
    }

    clearMessage();
    setBusy(true);
    dom.searchGo.disabled = true;
    clear(dom.hits);
    dom.hits.appendChild(make("p", "empty", "Searching…"));
    scheduleResize();

    try {
      // `query` alone. The schema is a oneOf over `query` XOR `vector`;
      // sending an empty `vector` alongside fails validation.
      var result = await window.greentic.invokeTool("rag_search", {
        query: query,
        top_k: topK,
      });
      renderHits(result && Array.isArray(result.hits) ? result.hits : [], query);
    } catch (err) {
      clear(dom.hits);
      dom.hits.appendChild(
        make(
          "p",
          "error",
          isTimeout(err)
            ? "The host stopped waiting after 10 seconds. Try the search again."
            : "Search failed: " + errorText(err)
        )
      );
    } finally {
      setBusy(false);
      dom.searchGo.disabled = false;
      scheduleResize();
    }
  }

  // ----------------------------------------------------------------- tabs

  function showTab(which) {
    var documents = which === "documents";
    dom.tabDocuments.classList.toggle("tab--active", documents);
    dom.tabSearch.classList.toggle("tab--active", !documents);
    dom.tabDocuments.setAttribute("aria-selected", String(documents));
    dom.tabSearch.setAttribute("aria-selected", String(!documents));
    dom.panelDocuments.hidden = !documents;
    dom.panelSearch.hidden = documents;
    scheduleResize();
  }

  // ------------------------------------------------------------- wiring

  function wire() {
    dom.tabDocuments.addEventListener("click", function () {
      showTab("documents");
    });
    dom.tabSearch.addEventListener("click", function () {
      showTab("search");
    });

    dom.file.addEventListener("change", function () {
      var files = dom.file.files;
      if (files && files.length > 0) {
        stageFile(files[0]);
      }
    });

    ["dragenter", "dragover"].forEach(function (name) {
      dom.drop.addEventListener(name, function (event) {
        event.preventDefault();
        dom.drop.classList.add("drop--over");
      });
    });
    ["dragleave", "drop"].forEach(function (name) {
      dom.drop.addEventListener(name, function () {
        dom.drop.classList.remove("drop--over");
      });
    });
    dom.drop.addEventListener("drop", function (event) {
      event.preventDefault();
      var files = event.dataTransfer && event.dataTransfer.files;
      if (files && files.length > 0) {
        stageFile(files[0]);
      }
    });

    dom.docId.addEventListener("input", renderReplaceNotice);
    dom.ingest.addEventListener("click", doIngest);
    dom.ingestCancel.addEventListener("click", function () {
      clearMessage();
      discardStaged();
    });

    dom.refresh.addEventListener("click", function () {
      loadPage(state.currentOffset);
    });
    dom.nextPage.addEventListener("click", goNextPage);
    dom.prevPage.addEventListener("click", goPrevPage);

    dom.searchForm.addEventListener("submit", doSearch);

    window.addEventListener("resize", scheduleResize);
  }

  // -------------------------------------------------------------- startup

  (async function start() {
    var init;
    try {
      init = await window.greentic.ready;
    } catch (err) {
      setStatus(
        "Could not connect to the host: " +
          errorText(err) +
          ". Nothing on this page can load until the host answers.",
        "error"
      );
      dom.app.setAttribute("aria-busy", "false");
      return;
    }

    state.connected = true;
    if (typeof init.locale === "string" && init.locale) {
      state.locale = init.locale;
      document.documentElement.lang = init.locale;
    }
    if (init.theme === "dark" || init.theme === "light") {
      document.documentElement.setAttribute("data-theme", init.theme);
    }

    setStatus(
      "Add documents here and they become searchable by every flow and agent " +
        "that uses this knowledge base.",
      "ok"
    );
    dom.app.setAttribute("aria-busy", "false");
    dom.workspace.hidden = false;

    wire();
    showTab("documents");
    await loadFirstPage();
    scheduleResize();
  })();
})();

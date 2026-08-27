/*
 * pdf.js — dependency-free, browser-only PDF text extractor.
 *
 * Ships verbatim inside a signed extension pack: no imports, no build step, no
 * remote code. Exposes exactly one global, `window.ragPdf`.
 *
 * Design rule that overrides everything else: the extracted text is ingested
 * into a knowledge base, so returning mojibake is far worse than returning
 * nothing. Whenever the decoding cannot be trusted we reject with a
 * plain-English explanation rather than guess.
 */
(function () {
  'use strict';

  /* ==================== byte / string helpers ==================== */

  function isWhite(c) {
    return c === 0x20 || c === 0x0a || c === 0x0d || c === 0x09 || c === 0x00 || c === 0x0c;
  }
  function isDelim(c) { // ( ) < > [ ] { } / %
    return c === 0x28 || c === 0x29 || c === 0x3c || c === 0x3e ||
      c === 0x5b || c === 0x5d || c === 0x7b || c === 0x7d || c === 0x2f || c === 0x25;
  }
  function isRegular(c) { return !isWhite(c) && !isDelim(c); }

  /** Latin-1 view of a byte range; the whole-file view is what we regex-scan. */
  function toLatin1(bytes) {
    var parts = [];
    for (var i = 0; i < bytes.length; i += 0x8000) {
      parts.push(String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000)));
    }
    return parts.join('');
  }

  function concatBytes(chunks, total) {
    var out = new Uint8Array(total), at = 0;
    for (var i = 0; i < chunks.length; i++) { out.set(chunks[i], at); at += chunks[i].length; }
    return out;
  }

  function indexOfBytes(bytes, needle, from) {
    var n = [];
    for (var i = 0; i < needle.length; i++) n.push(needle.charCodeAt(i));
    for (var p = from, limit = bytes.length - n.length; p <= limit; p++) {
      var ok = true;
      for (var j = 0; j < n.length; j++) { if (bytes[p + j] !== n[j]) { ok = false; break; } }
      if (ok) return p;
    }
    return -1;
  }

  // setTimeout rather than a microtask: the browser must get a chance to paint.
  function yieldToEventLoop() {
    return new Promise(function (resolve) { setTimeout(resolve, 0); });
  }

  /* ==================== PDF value types ==================== */

  function PdfName(name) { this.name = name; }
  function PdfRef(num, gen) { this.num = num; this.gen = gen; }
  function PdfString(bytes) { this.bytes = bytes; }
  function PdfStream(dict, raw) { this.dict = dict; this.raw = raw; }

  function PdfDict(map) { this.map = map; }
  PdfDict.prototype.get = function (k) {
    return Object.prototype.hasOwnProperty.call(this.map, k) ? this.map[k] : undefined;
  };
  PdfDict.prototype.has = function (k) {
    return Object.prototype.hasOwnProperty.call(this.map, k);
  };

  function isName(v, n) { return v instanceof PdfName && (n === undefined || v.name === n); }

  /* ==================== lexer / object parser ==================== */

  function Lexer(bytes, pos) { this.b = bytes; this.pos = pos || 0; }

  Lexer.prototype.skipWhite = function () {
    var b = this.b;
    while (this.pos < b.length) {
      var c = b[this.pos];
      if (isWhite(c)) { this.pos++; continue; }
      if (c === 0x25) { // '%' comment to end of line
        while (this.pos < b.length && b[this.pos] !== 0x0a && b[this.pos] !== 0x0d) this.pos++;
        continue;
      }
      break;
    }
  };

  /** { type, val } where type is num|name|str|arr_open|arr_close|dict_open|dict_close|kw|eof */
  Lexer.prototype.nextToken = function () {
    this.skipWhite();
    var b = this.b;
    if (this.pos >= b.length) return { type: 'eof' };
    var c = b[this.pos];
    if (c === 0x5b) { this.pos++; return { type: 'arr_open' }; }
    if (c === 0x5d) { this.pos++; return { type: 'arr_close' }; }
    if (c === 0x7b || c === 0x7d) { this.pos++; return { type: 'kw', val: String.fromCharCode(c) }; }
    if (c === 0x2f) return { type: 'name', val: this.readName() };
    if (c === 0x28) return { type: 'str', val: this.readLiteralString() };
    if (c === 0x3c) {
      if (b[this.pos + 1] === 0x3c) { this.pos += 2; return { type: 'dict_open' }; }
      return { type: 'str', val: this.readHexString() };
    }
    if (c === 0x3e) {
      if (b[this.pos + 1] === 0x3e) { this.pos += 2; return { type: 'dict_close' }; }
      this.pos++; // stray '>': skip rather than spin
      return this.nextToken();
    }
    if ((c >= 0x30 && c <= 0x39) || c === 0x2b || c === 0x2d || c === 0x2e) {
      return { type: 'num', val: this.readNumber() };
    }
    if (isRegular(c)) {
      var start = this.pos;
      while (this.pos < b.length && isRegular(b[this.pos])) this.pos++;
      return { type: 'kw', val: toLatin1(b.subarray(start, this.pos)) };
    }
    this.pos++;
    return this.nextToken();
  };

  Lexer.prototype.readNumber = function () {
    var b = this.b, start = this.pos;
    if (b[this.pos] === 0x2b || b[this.pos] === 0x2d) this.pos++;
    while (this.pos < b.length) {
      var c = b[this.pos];
      if ((c >= 0x30 && c <= 0x39) || c === 0x2e || c === 0x2d || c === 0x2b ||
        c === 0x65 || c === 0x45) { this.pos++; } else break;
    }
    var n = parseFloat(toLatin1(b.subarray(start, this.pos)));
    return isFinite(n) ? n : 0;
  };

  Lexer.prototype.readName = function () {
    var b = this.b, out = '';
    this.pos++; // '/'
    while (this.pos < b.length && isRegular(b[this.pos])) {
      var c = b[this.pos++];
      if (c === 0x23 && this.pos + 1 < b.length) { // '#xx' escape
        var hex = toLatin1(b.subarray(this.pos, this.pos + 2));
        if (/^[0-9a-fA-F]{2}$/.test(hex)) { out += String.fromCharCode(parseInt(hex, 16)); this.pos += 2; continue; }
      }
      out += String.fromCharCode(c);
    }
    return new PdfName(out);
  };

  Lexer.prototype.readLiteralString = function () {
    var b = this.b, out = [], depth = 1;
    this.pos++; // '('
    while (this.pos < b.length) {
      var c = b[this.pos++];
      if (c === 0x5c) { // escape
        if (this.pos >= b.length) break;
        var e = b[this.pos++];
        if (e === 0x6e) out.push(0x0a);
        else if (e === 0x72) out.push(0x0d);
        else if (e === 0x74) out.push(0x09);
        else if (e === 0x62) out.push(0x08);
        else if (e === 0x66) out.push(0x0c);
        else if (e === 0x28 || e === 0x29 || e === 0x5c) out.push(e);
        else if (e === 0x0d) { if (b[this.pos] === 0x0a) this.pos++; } // line continuation
        else if (e === 0x0a) { /* line continuation */ }
        else if (e >= 0x30 && e <= 0x37) { // octal \ddd, 1-3 digits
          var oct = e - 0x30;
          for (var k = 0; k < 2; k++) {
            var d = b[this.pos];
            if (d >= 0x30 && d <= 0x37) { oct = oct * 8 + (d - 0x30); this.pos++; } else break;
          }
          out.push(oct & 0xff);
        } else out.push(e); // unknown escape: the character itself
        continue;
      }
      if (c === 0x28) { depth++; out.push(c); continue; }
      if (c === 0x29) { if (--depth === 0) break; out.push(c); continue; }
      if (c === 0x0d) { if (b[this.pos] === 0x0a) this.pos++; out.push(0x0a); continue; }
      out.push(c);
    }
    return new PdfString(Uint8Array.from(out));
  };

  Lexer.prototype.readHexString = function () {
    var b = this.b, digits = '';
    this.pos++; // '<'
    while (this.pos < b.length) {
      var c = b[this.pos++];
      if (c === 0x3e) break;
      var ch = String.fromCharCode(c);
      if (/[0-9a-fA-F]/.test(ch)) digits += ch;
    }
    if (digits.length % 2 === 1) digits += '0'; // spec: pad the final nibble
    var out = new Uint8Array(digits.length / 2);
    for (var i = 0; i < out.length; i++) out[i] = parseInt(digits.substr(i * 2, 2), 16);
    return new PdfString(out);
  };

  /** Parse one object. `doc` is null for content streams (no indirect refs there). */
  Lexer.prototype.parseObject = function (tok, doc) {
    if (!tok) tok = this.nextToken();
    if (tok.type === 'num') {
      var save = this.pos, t2 = this.nextToken();
      if (t2.type === 'num' && Number.isInteger(tok.val) && tok.val >= 0) {
        var save2 = this.pos, t3 = this.nextToken();
        if (t3.type === 'kw' && t3.val === 'R') return new PdfRef(tok.val, t2.val);
        this.pos = save2;
      }
      this.pos = save;
      return tok.val;
    }
    if (tok.type === 'name' || tok.type === 'str') return tok.val;
    if (tok.type === 'arr_open') {
      var arr = [];
      for (;;) {
        var t = this.nextToken();
        if (t.type === 'arr_close' || t.type === 'eof' || t.type === 'dict_close') break;
        arr.push(this.parseObject(t, doc));
      }
      return arr;
    }
    if (tok.type === 'dict_open') {
      var map = Object.create(null);
      for (;;) {
        var kt = this.nextToken();
        if (kt.type === 'dict_close' || kt.type === 'eof') break;
        if (kt.type !== 'name') { this.parseObject(kt, doc); continue; }
        var vt = this.nextToken();
        if (vt.type === 'dict_close' || vt.type === 'eof') break;
        map[kt.val.name] = this.parseObject(vt, doc);
      }
      return this.maybeStream(new PdfDict(map), doc);
    }
    if (tok.type === 'kw') {
      if (tok.val === 'true') return true;
      if (tok.val === 'false') return false;
      if (tok.val === 'null') return null;
      return { keyword: tok.val };
    }
    return null;
  };

  /** After a dictionary, check for `stream` and capture the raw bytes. */
  Lexer.prototype.maybeStream = function (dict, doc) {
    var save = this.pos, t = this.nextToken();
    if (!(t.type === 'kw' && t.val === 'stream')) { this.pos = save; return dict; }
    var b = this.b;
    if (b[this.pos] === 0x0d) this.pos++; // data starts after CRLF or LF
    if (b[this.pos] === 0x0a) this.pos++;
    var start = this.pos, end = -1;

    var len = dict.get('Length');
    if (doc) len = doc.resolve(len);
    if (typeof len === 'number' && len >= 0 && start + len <= b.length) {
      // Trust /Length only when `endstream` really follows it; plenty of
      // writers emit a stale length after an edit.
      var p = start + len;
      while (p < b.length && isWhite(b[p])) p++;
      if (toLatin1(b.subarray(p, p + 9)) === 'endstream') end = start + len;
    }
    if (end < 0) {
      var idx = indexOfBytes(b, 'endstream', start);
      end = idx < 0 ? b.length : idx;
      if (end > start && b[end - 1] === 0x0a) end--;
      if (end > start && b[end - 1] === 0x0d) end--;
    }
    this.pos = Math.min(b.length, end);
    var after = indexOfBytes(b, 'endstream', this.pos);
    this.pos = after < 0 ? b.length : after + 9;
    return new PdfStream(dict, b.subarray(start, end));
  };

  /* ==================== stream filters ==================== */

  /** One pass through a DecompressionStream, keeping output from a truncated tail. */
  async function runDecompression(bytes, format) {
    var ds = new DecompressionStream(format);
    var writer = ds.writable.getWriter();
    // Not awaited on purpose: awaiting write() before reading deadlocks on
    // inputs larger than the transform's internal queue.
    writer.write(bytes).catch(function () {});
    writer.close().catch(function () {});
    var reader = ds.readable.getReader();
    var chunks = [], total = 0, error = null;
    for (;;) {
      var res;
      try { res = await reader.read(); } catch (e) { error = e; break; }
      if (res.done) break;
      chunks.push(res.value);
      total += res.value.length;
    }
    return { data: concatBytes(chunks, total), error: error };
  }

  /**
   * /FlateDecode. PDF nominally means zlib-wrapped deflate, but real writers
   * emit raw deflate and stray leading whitespace, so try the variants.
   */
  async function inflate(bytes, warnings, label) {
    if (bytes.length === 0) return new Uint8Array(0);
    var attempts = [[bytes, 'deflate'], [bytes, 'deflate-raw']];
    var lead = 0;
    while (lead < bytes.length && isWhite(bytes[lead])) lead++;
    if (lead > 0) attempts.push([bytes.subarray(lead), 'deflate'], [bytes.subarray(lead), 'deflate-raw']);

    var best = null;
    for (var i = 0; i < attempts.length; i++) {
      var r = await runDecompression(attempts[i][0], attempts[i][1]);
      if (!r.error && r.data.length > 0) return r.data;
      if (r.data.length > 0 && (!best || r.data.length > best.length)) best = r.data;
    }
    if (best) {
      warnings.push('A compressed stream' + (label ? ' (' + label + ')' : '') +
        ' was truncated or damaged; the recoverable part was used.');
      return best;
    }
    return null;
  }

  function asciiHexDecode(bytes) {
    var digits = '';
    for (var i = 0; i < bytes.length; i++) {
      var ch = String.fromCharCode(bytes[i]);
      if (ch === '>') break;
      if (/[0-9a-fA-F]/.test(ch)) digits += ch;
    }
    if (digits.length % 2 === 1) digits += '0';
    var out = new Uint8Array(digits.length / 2);
    for (var k = 0; k < out.length; k++) out[k] = parseInt(digits.substr(k * 2, 2), 16);
    return out;
  }

  function ascii85Decode(bytes) {
    var out = [], tuple = 0, count = 0, i = 0;
    if (bytes[0] === 0x3c && bytes[1] === 0x7e) i = 2; // optional '<~'
    for (; i < bytes.length; i++) {
      var c = bytes[i];
      if (isWhite(c)) continue;
      if (c === 0x7e) break; // '~>'
      if (c === 0x7a && count === 0) { out.push(0, 0, 0, 0); continue; } // 'z'
      if (c < 0x21 || c > 0x75) continue;
      tuple = tuple * 85 + (c - 0x21);
      if (++count === 5) {
        out.push((tuple >>> 24) & 0xff, (tuple >>> 16) & 0xff, (tuple >>> 8) & 0xff, tuple & 0xff);
        tuple = 0; count = 0;
      }
    }
    if (count > 0) {
      for (var p = count; p < 5; p++) tuple = tuple * 85 + 84;
      var full = [(tuple >>> 24) & 0xff, (tuple >>> 16) & 0xff, (tuple >>> 8) & 0xff, tuple & 0xff];
      for (var q = 0; q < count - 1; q++) out.push(full[q]);
    }
    return Uint8Array.from(out);
  }

  /**
   * PNG predictors (/Predictor >= 10). Cross-reference streams and object
   * streams are almost always predicted, so this is not optional.
   */
  function applyPngPredictor(data, colors, bpc, columns) {
    var bpp = Math.max(1, Math.ceil(colors * bpc / 8));
    var rowLen = Math.ceil(colors * bpc * columns / 8);
    var rows = Math.floor(data.length / (rowLen + 1));
    var out = new Uint8Array(rows * rowLen);
    var prev = new Uint8Array(rowLen);
    for (var r = 0; r < rows; r++) {
      var src = r * (rowLen + 1), type = data[src];
      var cur = out.subarray(r * rowLen, (r + 1) * rowLen);
      cur.set(data.subarray(src + 1, src + 1 + rowLen));
      for (var i = 0; i < rowLen; i++) {
        var a = i >= bpp ? cur[i - bpp] : 0, b = prev[i], c = i >= bpp ? prev[i - bpp] : 0;
        if (type === 1) cur[i] = (cur[i] + a) & 0xff;
        else if (type === 2) cur[i] = (cur[i] + b) & 0xff;
        else if (type === 3) cur[i] = (cur[i] + ((a + b) >> 1)) & 0xff;
        else if (type === 4) { // Paeth
          var p = a + b - c;
          var pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
          cur[i] = (cur[i] + ((pa <= pb && pa <= pc) ? a : (pb <= pc ? b : c))) & 0xff;
        }
      }
      prev = cur;
    }
    return out;
  }

  function maybeUnpredict(doc, data, pm) {
    if (!pm) return data;
    var predictor = doc.resolve(pm.get('Predictor')) || 1;
    if (predictor <= 1) return data;
    var colors = doc.resolve(pm.get('Colors')) || 1;
    var bpc = doc.resolve(pm.get('BitsPerComponent')) || 8;
    var columns = doc.resolve(pm.get('Columns')) || 1;
    // Only PNG predictors (>= 10) occur on the xref/objstm streams we read.
    return predictor >= 10 ? applyPngPredictor(data, colors, bpc, columns) : data;
  }

  /**
   * Decode a stream through its whole /Filter chain: { data } or
   * { unsupported: name } — never partial output presented as complete.
   * DCT/JPX/JBIG2/CCITT are images; LZW/RunLength are rare in modern text
   * PDFs, so warning beats hand-rolling a decoder for them.
   */
  async function decodeStream(doc, stream, warnings, label) {
    var data = stream.raw;
    var filters = doc.resolve(stream.dict.get('Filter'));
    if (filters === undefined || filters === null) filters = doc.resolve(stream.dict.get('F'));
    if (filters instanceof PdfName) filters = [filters];
    if (!Array.isArray(filters)) filters = [];

    var parms = doc.resolve(stream.dict.get('DecodeParms'));
    if (parms === undefined || parms === null) parms = doc.resolve(stream.dict.get('DP'));
    if (!Array.isArray(parms)) parms = [parms];

    for (var i = 0; i < filters.length; i++) {
      var f = doc.resolve(filters[i]);
      if (!(f instanceof PdfName)) continue;
      var pm = doc.resolve(parms[i]);
      if (!(pm instanceof PdfDict)) pm = null;

      if (f.name === 'FlateDecode' || f.name === 'Fl') {
        data = await inflate(data, warnings, label);
        if (!data) return { broken: true };
        data = maybeUnpredict(doc, data, pm);
      } else if (f.name === 'ASCIIHexDecode' || f.name === 'AHx') {
        data = asciiHexDecode(data);
      } else if (f.name === 'ASCII85Decode' || f.name === 'A85') {
        data = ascii85Decode(data);
      } else {
        return { unsupported: f.name };
      }
    }
    return { data: data };
  }

  /* ==================== document: objects, obj streams, trailer ==================== */

  function PdfDoc(bytes, warnings) {
    this.bytes = bytes;
    this.warnings = warnings;
    this.text = toLatin1(bytes);
    this.offsets = Object.create(null);  // objnum -> byte offset (last revision wins)
    this.entries = [];                   // sorted [{off, num}] for reverse lookup
    this.cache = Object.create(null);
    this.fromObjStm = Object.create(null);
    this.encrypted = false;
    this.rootRef = null;
  }

  /**
   * Build objnum -> offset by scanning the whole file for "N G obj".
   * We deliberately do NOT chase the xref table: real files routinely ship
   * stale or broken xrefs (bad offsets after a naive edit, hybrid files,
   * mismatched linearised first-page tables), whereas the literal "N G obj"
   * markers are always present and always authoritative.
   */
  PdfDoc.prototype.scanObjects = function () {
    var re = /(\d{1,10})\s+(\d{1,5})\s+obj\b/g, m;
    while ((m = re.exec(this.text)) !== null) {
      // Must start at a token boundary, else digits inside a binary blob match.
      var before = m.index > 0 ? this.text.charCodeAt(m.index - 1) : 0x0a;
      if (!isWhite(before) && !isDelim(before)) continue;
      var num = parseInt(m[1], 10);
      this.offsets[num] = m.index;
      this.entries.push({ off: m.index, num: num });
    }
    this.entries.sort(function (a, b) { return a.off - b.off; });
  };

  /** Object number whose body encloses `pos`, or null. */
  PdfDoc.prototype.objectNumberAt = function (pos) {
    var lo = 0, hi = this.entries.length - 1, best = -1;
    while (lo <= hi) {
      var mid = (lo + hi) >> 1;
      if (this.entries[mid].off <= pos) { best = mid; lo = mid + 1; } else hi = mid - 1;
    }
    if (best < 0) return null;
    var e = this.entries[best];
    if (this.offsets[e.num] !== e.off) return null; // a superseded revision
    return e.num;
  };

  PdfDoc.prototype.getObj = function (num) {
    if (Object.prototype.hasOwnProperty.call(this.cache, num)) return this.cache[num];
    if (Object.prototype.hasOwnProperty.call(this.fromObjStm, num)) return this.fromObjStm[num];
    var off = this.offsets[num];
    if (off === undefined) return null;
    this.cache[num] = null; // guards against reference cycles while parsing
    var lex = new Lexer(this.bytes, off);
    lex.nextToken(); lex.nextToken(); // object number, generation
    var kw = lex.nextToken();
    if (!(kw.type === 'kw' && kw.val === 'obj')) return null;
    var val = null;
    try { val = lex.parseObject(null, this); } catch (e) { val = null; }
    this.cache[num] = val;
    return val;
  };

  PdfDoc.prototype.resolve = function (v) {
    var guard = 0;
    while (v instanceof PdfRef && guard++ < 32) v = this.getObj(v.num);
    return v;
  };

  PdfDoc.prototype.dictOf = function (v) {
    v = this.resolve(v);
    if (v instanceof PdfStream) return v.dict;
    return v instanceof PdfDict ? v : null;
  };

  /**
   * Locate /Encrypt and /Root. Parsing every `trailer` dict plus every
   * cross-reference stream dict answers both questions without implementing
   * xref chasing.
   */
  PdfDoc.prototype.scanTrailers = function () {
    var self = this, seen = [], m;

    var re = /\btrailer\b/g;
    while ((m = re.exec(this.text)) !== null) {
      var d = null;
      try { d = new Lexer(this.bytes, m.index + 7).parseObject(null, this); } catch (e) { d = null; }
      if (d instanceof PdfDict) seen.push(d);
    }
    var reX = /\/Type\s*\/XRef\b/g;
    while ((m = reX.exec(this.text)) !== null) {
      var num = this.objectNumberAt(m.index);
      if (num === null) continue;
      var d2 = this.dictOf(this.getObj(num));
      if (d2) seen.push(d2);
    }
    seen.forEach(function (d) {
      if (d.has('Encrypt')) self.encrypted = true;
      if (!self.rootRef && d.has('Root')) self.rootRef = d.get('Root');
    });

    if (!this.encrypted) this.scanEncryptFallback();

    if (!this.rootRef) { // last resort: find the catalog object directly
      var reC = /\/Type\s*\/Catalog\b/g;
      while ((m = reC.exec(this.text)) !== null) {
        var cn = this.objectNumberAt(m.index);
        if (cn === null) continue;
        var cd = this.dictOf(this.getObj(cn));
        if (cd && isName(this.resolve(cd.get('Type')), 'Catalog')) {
          this.rootRef = new PdfRef(cn, 0);
          break;
        }
      }
    }
  };

  /**
   * Belt and braces: a `/Encrypt N G R` in plainly-ASCII surroundings (i.e.
   * not inside a compressed stream). Erring toward "encrypted" is right —
   * decryption is out of scope, and emitting ciphertext-derived mojibake
   * would be far worse than refusing.
   */
  PdfDoc.prototype.scanEncryptFallback = function () {
    var re = /\/Encrypt\s+\d+\s+\d+\s+R/g, m;
    while ((m = re.exec(this.text)) !== null) {
      var ctx = this.text.slice(Math.max(0, m.index - 120), m.index + 120);
      var printable = 0;
      for (var i = 0; i < ctx.length; i++) {
        var c = ctx.charCodeAt(i);
        if (c === 9 || c === 10 || c === 13 || (c >= 32 && c < 127)) printable++;
      }
      if (printable / ctx.length > 0.95) { this.encrypted = true; return; }
    }
  };

  /**
   * Expand /Type /ObjStm streams. Modern producers pack the catalog, page
   * dictionaries and font dictionaries into these, so without this step the
   * page tree is simply invisible.
   */
  PdfDoc.prototype.expandObjectStreams = async function (onProgress) {
    var candidates = [], seenNums = Object.create(null), m;
    var re = /\/ObjStm\b/g;
    while ((m = re.exec(this.text)) !== null) {
      var num = this.objectNumberAt(m.index);
      if (num === null || seenNums[num]) continue;
      seenNums[num] = true;
      candidates.push(num);
    }
    for (var i = 0; i < candidates.length; i++) {
      if (i % 8 === 7) await yieldToEventLoop();
      var obj = this.getObj(candidates[i]);
      if (!(obj instanceof PdfStream)) continue;
      if (!isName(this.resolve(obj.dict.get('Type')), 'ObjStm')) continue;
      var res = await decodeStream(this, obj, this.warnings, 'object stream');
      if (!res.data) {
        this.warnings.push('An object stream could not be decompressed; some content may be missing.');
        continue;
      }
      this.parseObjStm(res.data, obj.dict);
      if (onProgress) onProgress((i + 1) / candidates.length);
    }
  };

  PdfDoc.prototype.parseObjStm = function (data, dict) {
    var n = this.resolve(dict.get('N')) || 0;
    var first = this.resolve(dict.get('First')) || 0;
    var head = new Lexer(data, 0), pairs = [];
    for (var i = 0; i < n; i++) {
      var a = head.nextToken(), b = head.nextToken();
      if (a.type !== 'num' || b.type !== 'num') break;
      pairs.push([a.val, b.val]);
    }
    for (var k = 0; k < pairs.length; k++) {
      var objNum = pairs[k][0];
      // A directly-stored revision of the same object wins over the packed one.
      if (this.offsets[objNum] !== undefined) continue;
      if (Object.prototype.hasOwnProperty.call(this.fromObjStm, objNum)) continue;
      var val = null;
      try { val = new Lexer(data, first + pairs[k][1]).parseObject(null, this); } catch (e) { val = null; }
      this.fromObjStm[objNum] = val;
    }
  };

  /* ==================== page tree ==================== */

  var INHERITABLE = ['Resources', 'MediaBox', 'CropBox', 'Rotate'];

  function collectPages(doc, warnings) {
    var pages = [];
    var root = doc.dictOf(doc.rootRef);
    var pagesRoot = root ? doc.dictOf(root.get('Pages')) : null;
    if (pagesRoot) walkPageTree(doc, pagesRoot, Object.create(null), pages, 0, Object.create(null));

    if (pages.length === 0) {
      // Fallback: every object calling itself a page, in object-number order.
      // Only a heuristic, but it matches the write order of most producers.
      var nums = Object.keys(doc.offsets).map(Number);
      Object.keys(doc.fromObjStm).forEach(function (k) { nums.push(Number(k)); });
      nums.sort(function (a, b) { return a - b; });
      var seen = Object.create(null);
      nums.forEach(function (num) {
        if (seen[num]) return;
        seen[num] = true;
        var d = doc.dictOf(doc.getObj(num));
        if (d && isName(doc.resolve(d.get('Type')), 'Page')) {
          pages.push({ dict: d, inherited: collectInheritedByParent(doc, d) });
        }
      });
      if (pages.length > 0) {
        warnings.push('The document catalog could not be read; pages were recovered by ' +
          'scanning, so their order may not match the original.');
      }
    }
    return pages;
  }

  function walkPageTree(doc, node, inherited, out, depth, visited) {
    if (!node || depth > 64 || out.length > 5000) return;
    var merged = Object.create(null);
    INHERITABLE.forEach(function (k) { merged[k] = inherited[k]; });
    INHERITABLE.forEach(function (k) { if (node.has(k)) merged[k] = node.get(k); });

    var kids = doc.resolve(node.get('Kids'));
    if (Array.isArray(kids)) {
      for (var i = 0; i < kids.length; i++) {
        var ref = kids[i], key = ref instanceof PdfRef ? 'r' + ref.num : null;
        if (key) { if (visited[key]) continue; visited[key] = true; } // cyclic /Kids
        var kid = doc.dictOf(ref);
        if (kid) walkPageTree(doc, kid, merged, out, depth + 1, visited);
      }
      return;
    }
    if (isName(doc.resolve(node.get('Type')), 'Page') || (!kids && node.has('Contents'))) {
      out.push({ dict: node, inherited: merged });
    }
  }

  function collectInheritedByParent(doc, pageDict) {
    var merged = Object.create(null), chain = [], cur = pageDict, guard = 0;
    while (cur && guard++ < 32) { chain.push(cur); cur = doc.dictOf(cur.get('Parent')); }
    for (var i = chain.length - 1; i >= 0; i--) {
      INHERITABLE.forEach(function (k) { if (chain[i].has(k)) merged[k] = chain[i].get(k); });
    }
    return merged;
  }

  /* ==================== glyph name -> Unicode ==================== */

  var GLYPH_TABLE = (function () {
    var t = Object.create(null), upper = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ';
    for (var i = 0; i < 26; i++) { t[upper[i]] = 0x41 + i; t[upper[i].toLowerCase()] = 0x61 + i; }
    ['zero', 'one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight', 'nine']
      .forEach(function (n, d) { t[n] = 0x30 + d; });

    // name/hex pairs, space separated.
    var packed =
      'space 20 exclam 21 quotedbl 22 numbersign 23 dollar 24 percent 25 ampersand 26 quotesingle 27 quoteright 2019 parenleft 28 parenright 29 asterisk 2A plus 2B comma 2C ' +
      'hyphen 2D period 2E slash 2F colon 3A semicolon 3B less 3C equal 3D greater 3E question 3F at 40 bracketleft 5B backslash 5C bracketright 5D asciicircum 5E underscore 5F ' +
      'grave 60 quoteleft 2018 braceleft 7B bar 7C braceright 7D asciitilde 7E exclamdown A1 cent A2 sterling A3 fraction 2044 currency A4 yen A5 florin 192 section A7 ' +
      'currency1 A4 quotesingbase 201A quotedblbase 201E quotedblleft 201C quotedblright 201D guilsinglleft 2039 guilsinglright 203A guillemotleft AB guillemotright BB ' +
      'fi FB01 fl FB02 ff FB00 ffi FB03 ffl FB04 endash 2013 emdash 2014 figuredash 2012 dagger 2020 daggerdbl 2021 periodcentered B7 middot B7 bullet 2022 ellipsis 2026 ' +
      'perthousand 2030 questiondown BF quotedbllen 201D circumflex 2C6 tilde 2DC macron AF breve 2D8 dotaccent 2D9 dieresis A8 ring 2DA cedilla B8 hungarumlaut 2DD ogonek 2DB caron 2C7 ' +
      'acute B4 degree B0 minus 2212 multiply D7 divide F7 plusminus B1 notequal 2260 lessequal 2264 greaterequal 2265 approxequal 2248 infinity 221E integral 222B ' +
      'partialdiff 2202 product 220F summation 2211 radical 221A logicalnot AC mu B5 micro B5 Omega 2126 Delta 2206 pi 3C0 lozenge 25CA Euro 20AC euro 20AC copyright A9 registered AE ' +
      'trademark 2122 ordfeminine AA ordmasculine BA paragraph B6 germandbls DF onesuperior B9 twosuperior B2 threesuperior B3 onehalf BD onequarter BC threequarters BE ' +
      'brokenbar A6 hyphensoft AD softhyphen AD nbspace A0 space1 A0 AE C6 ae E6 Oslash D8 oslash F8 OE 152 oe 153 Lslash 141 lslash 142 Eth D0 eth F0 Thorn DE thorn FE Dcroat 110 dcroat 111 ' +
      'Aacute C1 Acircumflex C2 Adieresis C4 Agrave C0 Aring C5 Atilde C3 Amacron 100 Abreve 102 Aogonek 104 aacute E1 acircumflex E2 adieresis E4 agrave E0 aring E5 atilde E3 amacron 101 abreve 103 aogonek 105 ' +
      'Ccedilla C7 ccedilla E7 Cacute 106 cacute 107 Ccaron 10C ccaron 10D Dcaron 10E dcaron 10F Eacute C9 Ecircumflex CA Edieresis CB Egrave C8 Emacron 112 Ecaron 11A Eogonek 118 Edotaccent 116 ' +
      'eacute E9 ecircumflex EA edieresis EB egrave E8 emacron 113 ecaron 11B eogonek 119 edotaccent 117 Gbreve 11E gbreve 11F Iacute CD Icircumflex CE Idieresis CF Igrave CC Imacron 12A Iogonek 12E Idotaccent 130 ' +
      'iacute ED icircumflex EE idieresis EF igrave EC imacron 12B iogonek 12F dotlessi 131 Lacute 139 lacute 13A Lcaron 13D lcaron 13E Nacute 143 nacute 144 Ncaron 147 ncaron 148 ' +
      'Ntilde D1 ntilde F1 Oacute D3 Ocircumflex D4 Odieresis D6 Ograve D2 Otilde D5 Ohungarumlaut 150 Omacron 14C oacute F3 ocircumflex F4 odieresis F6 ograve F2 otilde F5 ohungarumlaut 151 omacron 14D ' +
      'Racute 154 racute 155 Rcaron 158 rcaron 159 Sacute 15A sacute 15B Scaron 160 scaron 161 Scedilla 15E scedilla 15F Tcaron 164 tcaron 165 Tcommaaccent 162 tcommaaccent 163 ' +
      'Uacute DA Ucircumflex DB Udieresis DC Ugrave D9 Uhungarumlaut 170 Umacron 16A Uring 16E Uogonek 172 uacute FA ucircumflex FB udieresis FC ugrave F9 uhungarumlaut 171 umacron 16B uring 16F uogonek 173 ' +
      'Yacute DD Ydieresis 178 yacute FD ydieresis FF Zacute 179 zacute 17A Zcaron 17D zcaron 17E Zdotaccent 17B zdotaccent 17C Scommaaccent 218 scommaaccent 219 ' +
      'arrowleft 2190 arrowup 2191 arrowright 2192 arrowdown 2193 arrowboth 2194 arrowdblright 21D2 arrowdblleft 21D0 arrowdblboth 21D4 element 2208 notelement 2209 intersection 2229 union 222A ' +
      'propersubset 2282 propersuperset 2283 existential 2203 universal 2200 emptyset 2205 gradient 2207 angle 2220 equivalence 2261 proportional 221D therefore 2234 similar 223C congruent 2245 ' +
      'alpha 3B1 beta 3B2 gamma 3B3 delta 3B4 epsilon 3B5 zeta 3B6 eta 3B7 theta 3B8 iota 3B9 kappa 3BA lambda 3BB nu 3BD xi 3BE omicron 3BF rho 3C1 sigma 3C3 tau 3C4 upsilon 3C5 phi 3C6 chi 3C7 psi 3C8 omega 3C9 ' +
      'Gamma 393 Theta 398 Lambda 39B Xi 39E Pi 3A0 Sigma 3A3 Upsilon 3A5 Phi 3A6 Psi 3A8 commaaccent 2E onedotenleader 2024 twodotenleader 2025 zerooldstyle 30 numbersignsmall 23 asciitildelow 7E nonbreakingspace A0';
    var parts = packed.split(' ');
    for (var p = 0; p + 1 < parts.length; p += 2) {
      if (t[parts[p]] === undefined) t[parts[p]] = parseInt(parts[p + 1], 16);
    }
    return t;
  })();

  /**
   * WinAnsi (CP1252) codes 0x80-0x9F; the rest of its high range is plain
   * Latin-1. The five unassigned slots are written as escapes on purpose —
   * as literal control characters they are invisible and get silently lost
   * when the table is reflowed, which shifts every glyph after them.
   */
  var WINANSI_HIGH =
    '\u20AC\u0081\u201A\u0192\u201E\u2026\u2020\u2021\u02C6\u2030\u0160\u2039\u0152\u008D\u017D\u008F' +
    '\u0090\u2018\u2019\u201C\u201D\u2022\u2013\u2014\u02DC\u2122\u0161\u203A\u0153\u009D\u017E\u0178';

  /** MacRomanEncoding, codes 0x80-0xFF (exactly 128 entries). */
  var MACROMAN_HIGH =
    'ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü' +
    '†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø' +
    '¿¡¬√ƒ≈∆«»…\u00A0ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ' +
    '‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ\uF8FFÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ';

  /** StandardEncoding high range (sparse), as code/codepoint hex pairs. */
  var STANDARD_HIGH = (function () {
    var t = Object.create(null);
    var packed = 'A1 00A1 A2 00A2 A3 00A3 A4 2044 A5 00A5 A6 0192 A7 00A7 A8 00A4 A9 0027 ' +
      'AA 201C AB 00AB AC 2039 AD 203A AE FB01 AF FB02 B1 2013 B2 2020 B3 2021 B4 00B7 ' +
      'B6 00B6 B7 2022 B8 201A B9 201E BA 201D BB 00BB BC 2026 BD 2030 BF 00BF ' +
      'C1 0060 C2 00B4 C3 02C6 C4 02DC C5 00AF C6 02D8 C7 02D9 C8 00A8 CA 02DA CB 00B8 ' +
      'CD 02DD CE 02DB CF 02C7 D0 2014 E1 00C6 E3 00AA E8 0141 E9 00D8 EA 0152 EB 00BA ' +
      'F1 00E6 F5 0131 F8 0142 F9 00F8 FA 0153 FB 00DF';
    var p = packed.split(' ');
    for (var i = 0; i + 1 < p.length; i += 2) t[parseInt(p[i], 16)] = String.fromCharCode(parseInt(p[i + 1], 16));
    return t;
  })();

  function winAnsiToUnicode(code) {
    if (code >= 0x20 && code <= 0x7e) return String.fromCharCode(code);
    if (code >= 0x80 && code <= 0x9f) return WINANSI_HIGH.charAt(code - 0x80);
    if (code >= 0xa0 && code <= 0xff) return String.fromCharCode(code);
    return null;
  }

  function macRomanToUnicode(code) {
    if (code >= 0x20 && code <= 0x7e) return String.fromCharCode(code);
    if (code >= 0x80) return MACROMAN_HIGH.charAt(code - 0x80);
    return null;
  }

  function standardToUnicode(code) {
    if (code === 0x27) return '’'; // quoteright, not the ASCII apostrophe
    if (code === 0x60) return '‘'; // quoteleft, not a grave accent
    if (code >= 0x20 && code <= 0x7e) return String.fromCharCode(code);
    var high = STANDARD_HIGH[code];
    return high !== undefined ? high : null;
  }

  function safeFromCodePoint(cp) {
    if (!isFinite(cp) || cp < 0 || cp > 0x10ffff) return null;
    if (cp >= 0xd800 && cp <= 0xdfff) return null;
    try { return String.fromCodePoint(cp); } catch (e) { return null; }
  }

  /** Glyph name -> Unicode: the Standard/WinAnsi/MacRoman name set plus the
   *  uniXXXX / uXXXXXX conventions. gNN / cidNN carry no Unicode meaning. */
  function glyphNameToUnicode(name) {
    if (!name) return null;
    if (GLYPH_TABLE[name] !== undefined) return String.fromCodePoint(GLYPH_TABLE[name]);
    var m = /^uni([0-9A-Fa-f]{4,6})$/.exec(name) || /^u([0-9A-Fa-f]{4,6})$/.exec(name);
    if (m) return safeFromCodePoint(parseInt(m[1], 16));
    if (name.indexOf('_') > 0) { // ligature names such as "f_i"
      var parts = name.split('_'), out = '';
      for (var i = 0; i < parts.length; i++) {
        var piece = glyphNameToUnicode(parts[i]);
        if (piece === null) return null;
        out += piece;
      }
      return out;
    }
    var dot = name.indexOf('.'); // variant suffixes such as "A.sc"
    if (dot > 0) return glyphNameToUnicode(name.slice(0, dot));
    return null;
  }

  /* ==================== fonts ==================== */

  function utf16beToString(bytes) {
    if (bytes.length === 1) return String.fromCharCode(bytes[0]);
    var s = '';
    for (var i = 0; i + 1 < bytes.length; i += 2) s += String.fromCharCode((bytes[i] << 8) | bytes[i + 1]);
    return s;
  }

  function bytesToCode(bytes) {
    var v = 0;
    for (var i = 0; i < bytes.length; i++) v = (v << 8) | bytes[i];
    return v >>> 0;
  }

  function bfDestination(dst) {
    if (dst instanceof PdfString) {
      var s = utf16beToString(dst.bytes);
      return s.length ? s : null;
    }
    if (dst instanceof PdfName) return glyphNameToUnicode(dst.name);
    return null;
  }

  /**
   * Parse a /ToUnicode CMap: begincodespacerange (which gives the code
   * width), beginbfchar and beginbfrange (array form and multi-unit UTF-16BE
   * destinations, i.e. ligatures, included). The only fully reliable
   * glyph->Unicode source in a PDF, so it outranks every other mapping.
   */
  function parseToUnicodeCMap(data) {
    var lex = new Lexer(data, 0);
    var map = Object.create(null), codeLengths = Object.create(null);
    var stack = [], count = 0;

    for (;;) {
      var t = lex.nextToken();
      if (t.type === 'eof' || ++count > 2000000) break;

      if (t.type !== 'kw') {
        stack.push(lex.parseObject(t, null));
        if (stack.length > 30000) stack = stack.slice(-30000);
        continue;
      }
      var op = t.val, i;
      if (op === 'endcodespacerange') {
        for (i = 0; i + 1 < stack.length; i += 2) {
          if (stack[i] instanceof PdfString) codeLengths[stack[i].bytes.length] = true;
        }
      } else if (op === 'endbfchar') {
        for (i = 0; i + 1 < stack.length; i += 2) {
          var src = stack[i];
          if (!(src instanceof PdfString)) continue;
          codeLengths[src.bytes.length] = true;
          var text = bfDestination(stack[i + 1]);
          if (text !== null) map[bytesToCode(src.bytes)] = text;
        }
      } else if (op === 'endbfrange') {
        for (i = 0; i + 2 < stack.length; i += 3) {
          var lo = stack[i], hi = stack[i + 1], d = stack[i + 2];
          if (!(lo instanceof PdfString) || !(hi instanceof PdfString)) continue;
          codeLengths[lo.bytes.length] = true;
          var loC = bytesToCode(lo.bytes), hiC = bytesToCode(hi.bytes);
          if (hiC < loC || hiC - loC > 65535) continue;
          if (Array.isArray(d)) {
            for (var a = 0; a <= hiC - loC && a < d.length; a++) {
              var av = bfDestination(d[a]);
              if (av !== null) map[loC + a] = av;
            }
          } else if (d instanceof PdfString) {
            var base = utf16beToString(d.bytes);
            if (base.length === 0) continue;
            var prefix = base.slice(0, base.length - 1);
            var lastUnit = base.charCodeAt(base.length - 1);
            for (var b = 0; b <= hiC - loC; b++) {
              map[loC + b] = prefix + String.fromCharCode((lastUnit + b) & 0xffff);
            }
          }
        }
      }
      stack = [];
    }
    return { map: map, codeLengths: Object.keys(codeLengths).map(Number) };
  }

  /** /W array: [ c [w1 w2 ...] ] or [ cFirst cLast w ] */
  function parseCidWidths(doc, w) {
    if (!Array.isArray(w)) return null;
    var out = Object.create(null), i = 0;
    while (i < w.length) {
      var first = doc.resolve(w[i]);
      if (typeof first !== 'number') { i++; continue; }
      var next = doc.resolve(w[i + 1]);
      if (Array.isArray(next)) {
        for (var k = 0; k < next.length; k++) {
          var v = doc.resolve(next[k]);
          if (typeof v === 'number') out[first + k] = v;
        }
        i += 2;
      } else if (typeof next === 'number') {
        var val = doc.resolve(w[i + 2]);
        if (typeof val === 'number' && next >= first && next - first < 70000) {
          for (var c = first; c <= next; c++) out[c] = val;
        }
        i += 3;
      } else i++;
    }
    return out;
  }

  /** Build a decoder for one font dictionary. */
  async function loadFont(doc, fontDict, warnings) {
    var subtype = doc.resolve(fontDict.get('Subtype'));
    var isType0 = isName(subtype, 'Type0');
    var baseFont = doc.resolve(fontDict.get('BaseFont'));
    var fontLabel = baseFont instanceof PdfName ? baseFont.name : '(unnamed font)';

    var font = {
      label: fontLabel, twoByte: isType0, toUnicode: null,
      differences: Object.create(null), baseEncoding: null,
      widths: null, firstChar: 0, defaultWidth: isType0 ? 1000 : 500,
      // Glyph-space -> text-space factor for /Widths: 1/1000 for every font
      // type except Type3, whose widths are in its own /FontMatrix space.
      widthScale: 0.001, cidWidths: null
    };

    // ---- 1. /ToUnicode wins outright when present.
    var tu = doc.resolve(fontDict.get('ToUnicode'));
    if (tu instanceof PdfStream) {
      var res = await decodeStream(doc, tu, warnings, 'ToUnicode CMap');
      if (res.data) {
        var parsed = parseToUnicodeCMap(res.data);
        if (Object.keys(parsed.map).length > 0) font.toUnicode = parsed.map;
        if (parsed.codeLengths.indexOf(2) >= 0) font.twoByte = true;
        else if (parsed.codeLengths.length === 1 && parsed.codeLengths[0] === 1) font.twoByte = false;
      }
    }

    // ---- 2. /Encoding
    var enc = doc.resolve(fontDict.get('Encoding'));
    if (isType0) {
      if (enc instanceof PdfName && /^Identity-[HV]$/.test(enc.name)) font.twoByte = true;
      else if (enc instanceof PdfStream) font.twoByte = true;
      // Identity-H means "code == CID", and a CID is a font-internal glyph
      // index with no Unicode meaning. Without /ToUnicode there is nothing
      // honest to map it to, so flag it and let the garbage heuristic decide.
      if (!font.toUnicode) {
        warnings.push('Font "' + fontLabel + '" uses a composite encoding with no /ToUnicode ' +
          'map, so its characters cannot be decoded.');
      }
    } else if (enc instanceof PdfName) {
      font.baseEncoding = enc.name;
    } else if (enc instanceof PdfDict) {
      var be = doc.resolve(enc.get('BaseEncoding'));
      if (be instanceof PdfName) font.baseEncoding = be.name;
      var diffs = doc.resolve(enc.get('Differences'));
      if (Array.isArray(diffs)) {
        var code = 0;
        for (var i = 0; i < diffs.length; i++) {
          var item = doc.resolve(diffs[i]);
          if (typeof item === 'number') code = item | 0;
          else if (item instanceof PdfName) font.differences[code++] = item.name;
        }
      }
    }

    // ---- 3. Widths — used only for spacing decisions, never for decoding.
    if (isType0) {
      var desc = doc.resolve(fontDict.get('DescendantFonts'));
      var d0 = Array.isArray(desc) ? doc.dictOf(desc[0]) : null;
      if (d0) {
        var dw = doc.resolve(d0.get('DW'));
        font.defaultWidth = typeof dw === 'number' ? dw : 1000;
        font.cidWidths = parseCidWidths(doc, doc.resolve(d0.get('W')));
      }
    } else {
      var w = doc.resolve(fontDict.get('Widths'));
      if (Array.isArray(w)) {
        font.widths = w.map(function (x) { var v = doc.resolve(x); return typeof v === 'number' ? v : 0; });
        var fc = doc.resolve(fontDict.get('FirstChar'));
        font.firstChar = typeof fc === 'number' ? fc : 0;
      }
      var fd = doc.dictOf(fontDict.get('FontDescriptor'));
      if (fd) {
        var mw = doc.resolve(fd.get('MissingWidth'));
        if (typeof mw === 'number') font.defaultWidth = mw;
      }
      if (isName(subtype, 'Type3')) {
        var fm = doc.resolve(fontDict.get('FontMatrix'));
        var sx = (Array.isArray(fm) && typeof doc.resolve(fm[0]) === 'number') ? doc.resolve(fm[0]) : 0.001;
        font.widthScale = sx || 0.001;
        // Type3 has no standard default width, and a half-em guess would be
        // in the wrong unit system anyway.
        font.defaultWidth = 0;
      }
    }

    font.unicodeOf = function (code) {
      if (font.toUnicode) {
        var mapped = font.toUnicode[code];
        if (mapped !== undefined) return mapped;
      }
      if (font.twoByte) return null; // composite font, unmapped code
      var glyph = font.differences[code];
      if (glyph !== undefined) {
        var viaName = glyphNameToUnicode(glyph);
        if (viaName !== null) return viaName;
      }
      var base = font.baseEncoding;
      if (base === 'WinAnsiEncoding') return winAnsiToUnicode(code);
      if (base === 'MacRomanEncoding') return macRomanToUnicode(code);
      if (base === 'StandardEncoding' || base === 'MacExpertEncoding' ||
        base === null || base === undefined) {
        var std = standardToUnicode(code);
        // WinAnsi is the fallback for 0x80-0x9F, where Standard is blank.
        return std !== null ? std : winAnsiToUnicode(code);
      }
      return winAnsiToUnicode(code);
    };

    font.widthOf = function (code) {
      if (font.cidWidths) {
        var cw = font.cidWidths[code];
        return (cw === undefined ? font.defaultWidth : cw) * 0.001;
      }
      if (font.widths) {
        var idx = code - font.firstChar;
        if (idx >= 0 && idx < font.widths.length && font.widths[idx] > 0) {
          return font.widths[idx] * font.widthScale;
        }
      }
      return font.defaultWidth * font.widthScale;
    };

    return font;
  }

  /* ==================== content stream text extraction ==================== */

  var IDENTITY = [1, 0, 0, 1, 0, 0];

  /** result = m applied, then n. */
  function mul(m, n) {
    return [
      m[0] * n[0] + m[1] * n[2], m[0] * n[1] + m[1] * n[3],
      m[2] * n[0] + m[3] * n[2], m[2] * n[1] + m[3] * n[3],
      m[4] * n[0] + m[5] * n[2] + n[4], m[4] * n[1] + m[5] * n[3] + n[5]
    ];
  }

  function TextSink() {
    this.parts = [];
    this.started = false;
    this.lastX = 0;
    this.lastY = 0;
    this.lineStartX = 0;
  }
  TextSink.prototype.push = function (s) { if (s) this.parts.push(s); };
  TextSink.prototype.lastChar = function () {
    for (var i = this.parts.length - 1; i >= 0; i--) {
      if (this.parts[i].length) return this.parts[i].charAt(this.parts[i].length - 1);
    }
    return '';
  };
  TextSink.prototype.toText = function () { return this.parts.join(''); };

  /**
   * Inline images (BI ... ID <binary> EI) carry raw binary that would derail
   * the lexer, so skip from ID to the next whitespace-delimited EI.
   */
  function skipInlineImage(lex) {
    var b = lex.b, idAt = -1;
    for (var p = lex.pos; p + 1 < b.length; p++) {
      if (b[p] === 0x49 && b[p + 1] === 0x44 && (p === 0 || !isRegular(b[p - 1]))) { idAt = p; break; }
      if (b[p] === 0x45 && b[p + 1] === 0x49) { lex.pos = p + 2; return; }
    }
    if (idAt < 0) return;
    for (var q = idAt + 2; q + 1 < b.length; q++) {
      if (b[q] === 0x45 && b[q + 1] === 0x49 && isWhite(b[q - 1]) &&
        (q + 2 >= b.length || !isRegular(b[q + 2]))) { lex.pos = q + 2; return; }
    }
    lex.pos = b.length;
  }

  async function runContent(doc, data, resources, sink, warnings, fontCache, initialCtm, depth) {
    var lex = new Lexer(data, 0);
    var operands = [], ctm = initialCtm, ctmStack = [];
    var tm = IDENTITY.slice(), tlm = IDENTITY.slice();
    var leading = 0, fontSize = 0, charSpacing = 0, wordSpacing = 0, horizScale = 1;
    var font = null, ops = 0;

    var fontRes = resources ? doc.dictOf(resources.get('Font')) : null;
    var xobjRes = resources ? doc.dictOf(resources.get('XObject')) : null;

    function devX() { return tm[4] * ctm[0] + tm[5] * ctm[2] + ctm[4]; }
    function devY() { return tm[4] * ctm[1] + tm[5] * ctm[3] + ctm[5]; }
    function ctmScale() {
      var sx = Math.sqrt(ctm[0] * ctm[0] + ctm[1] * ctm[1]);
      var sy = Math.sqrt(ctm[2] * ctm[2] + ctm[3] * ctm[3]);
      return (sx + sy) / 2 || 1;
    }

    /** Split a shown string into codes using the font's code width. */
    function codesOf(bytes) {
      var out = [], i;
      if (font && font.twoByte) {
        for (i = 0; i + 1 < bytes.length; i += 2) out.push((bytes[i] << 8) | bytes[i + 1]);
        if (bytes.length % 2 === 1) out.push(bytes[bytes.length - 1]);
      } else {
        for (i = 0; i < bytes.length; i++) out.push(bytes[i]);
      }
      return out;
    }

    /** Decide whether a newline or a space precedes the text about to be shown. */
    function positionBreak() {
      var x = devX(), y = devY(), scale = ctmScale();
      var effSize = Math.abs(fontSize) * scale || 1;
      var lineH = Math.max(effSize, Math.abs(leading) * scale, 1);

      if (!sink.started) {
        sink.started = true;
        sink.lastX = sink.lineStartX = x;
        sink.lastY = y;
        return;
      }
      var dy = sink.lastY - y, gap = x - sink.lastX, last = sink.lastChar();
      if (Math.abs(dy) > 0.4 * lineH) {
        sink.push(dy > 1.9 * lineH ? '\n\n' : '\n');
        sink.lineStartX = x;
      } else if (gap < -0.5 * effSize && x <= sink.lineStartX + 0.5 * effSize) {
        // Same baseline, but the pen went back to where this line began: a
        // producer resetting the line matrix instead of using T*. The
        // lineStartX guard matters — advance widths are estimates, so small
        // backwards drift mid-line is normal and is not a line break.
        if (last !== '\n') sink.push('\n');
        sink.lineStartX = x;
      } else if (gap > 0.22 * effSize && last !== '' && last !== ' ' && last !== '\n') {
        sink.push(' ');
      }
      sink.lastY = y;
    }

    /** U+FFFD is deliberate: an undecodable glyph must stay visible to the
     *  garbage heuristic so the document as a whole gets rejected. */
    function decodeCodes(codes) {
      var text = '', advance = 0;
      for (var i = 0; i < codes.length; i++) {
        var u = font.unicodeOf(codes[i]);
        text += (u === null || u === undefined) ? '�' : u;
        var w = font.widthOf(codes[i]) * fontSize + charSpacing;
        if (!font.twoByte && codes[i] === 32) w += wordSpacing;
        advance += w;
      }
      return { text: text, advance: advance };
    }

    function ensureFont() {
      // No Tf seen: assume a 1-byte WinAnsi font rather than dropping the text.
      if (!font) {
        font = {
          twoByte: false, label: '(implicit)',
          unicodeOf: winAnsiToUnicode, widthOf: function () { return 0.5; }
        };
      }
    }

    function showString(bytes) {
      ensureFont();
      positionBreak();
      var r = decodeCodes(codesOf(bytes));
      sink.push(r.text);
      tm = mul([1, 0, 0, 1, r.advance * horizScale, 0], tm);
      sink.lastX = devX();
      sink.lastY = devY();
    }

    function applyTJ(arr) {
      ensureFont();
      positionBreak();
      var text = '';
      for (var i = 0; i < arr.length; i++) {
        var el = arr[i];
        if (typeof el === 'number') {
          // A large negative adjustment is how most producers encode a space.
          if (el < -100 && text.length && text.charAt(text.length - 1) !== ' ') text += ' ';
          tm = mul([1, 0, 0, 1, -el / 1000 * fontSize * horizScale, 0], tm);
        } else if (el instanceof PdfString) {
          var r = decodeCodes(codesOf(el.bytes));
          text += r.text;
          tm = mul([1, 0, 0, 1, r.advance * horizScale, 0], tm);
        }
      }
      sink.push(text);
      sink.lastX = devX();
      sink.lastY = devY();
    }

    function nextLine(tx, ty) {
      tlm = mul([1, 0, 0, 1, tx, ty], tlm);
      tm = tlm.slice();
    }

    for (;;) {
      var tok = lex.nextToken();
      if (tok.type === 'eof') break;
      if (++ops > 4000000) {
        warnings.push('A page had an unusually long content stream and was truncated.');
        break;
      }
      if (tok.type !== 'kw') {
        operands.push(lex.parseObject(tok, null));
        if (operands.length > 64) operands = operands.slice(-64);
        continue;
      }

      var op = tok.val, n = operands.length;
      var num = function (i) { var v = operands[n - i]; return typeof v === 'number' ? v : 0; };

      if (op === 'BT') { tm = IDENTITY.slice(); tlm = IDENTITY.slice(); }
      else if (op === 'q') ctmStack.push(ctm.slice());
      else if (op === 'Q') { if (ctmStack.length) ctm = ctmStack.pop(); }
      else if (op === 'cm') { if (n >= 6) ctm = mul([num(6), num(5), num(4), num(3), num(2), num(1)], ctm); }
      else if (op === 'TL') leading = num(1);
      else if (op === 'Tc') charSpacing = num(1);
      else if (op === 'Tw') wordSpacing = num(1);
      else if (op === 'Tz') horizScale = num(1) / 100 || 1;
      else if (op === 'Td') { if (n >= 2) nextLine(num(2), num(1)); }
      else if (op === 'TD') { if (n >= 2) { leading = -num(1); nextLine(num(2), num(1)); } }
      else if (op === 'Tm') { if (n >= 6) { tlm = [num(6), num(5), num(4), num(3), num(2), num(1)]; tm = tlm.slice(); } }
      else if (op === 'T*') nextLine(0, -leading);
      else if (op === 'Tj') { if (operands[n - 1] instanceof PdfString) showString(operands[n - 1].bytes); }
      else if (op === 'TJ') { if (Array.isArray(operands[n - 1])) applyTJ(operands[n - 1]); }
      else if (op === "'") {
        nextLine(0, -leading);
        if (operands[n - 1] instanceof PdfString) showString(operands[n - 1].bytes);
      } else if (op === '"') {
        if (n >= 3) { wordSpacing = num(3); charSpacing = num(2); }
        nextLine(0, -leading);
        if (operands[n - 1] instanceof PdfString) showString(operands[n - 1].bytes);
      } else if (op === 'BI') {
        skipInlineImage(lex);
      } else if (op === 'Tf') {
        fontSize = num(1);
        var nameOp = operands[n - 2];
        font = null;
        if (nameOp instanceof PdfName && fontRes) {
          var fdictRef = fontRes.get(nameOp.name);
          var cacheKey = (fdictRef instanceof PdfRef) ? 'r' + fdictRef.num : null;
          if (cacheKey && fontCache[cacheKey]) {
            font = fontCache[cacheKey];
          } else {
            var fdict = doc.dictOf(fdictRef);
            if (fdict) {
              font = await loadFont(doc, fdict, warnings);
              if (cacheKey) fontCache[cacheKey] = font;
            }
          }
        }
      } else if (op === 'Do') {
        // Form XObjects legitimately hold page text (headers, tables, anything
        // a producer factored out), so recurse a bounded amount.
        var xname = operands[n - 1];
        if (depth < 4 && xname instanceof PdfName && xobjRes) {
          var xo = doc.resolve(xobjRes.get(xname.name));
          if (xo instanceof PdfStream && isName(doc.resolve(xo.dict.get('Subtype')), 'Form')) {
            var xres = await decodeStream(doc, xo, warnings, 'form XObject');
            if (xres.data) {
              var mtx = doc.resolve(xo.dict.get('Matrix'));
              var childCtm = (Array.isArray(mtx) && mtx.length === 6) ? mul(mtx, ctm) : ctm.slice();
              await runContent(doc, xres.data, doc.dictOf(xo.dict.get('Resources')) || resources,
                sink, warnings, fontCache, childCtm, depth + 1);
            }
          }
        }
      }
      operands = [];
    }
  }

  async function extractPageText(doc, page, warnings, fontCache) {
    var contents = doc.resolve(page.dict.get('Contents'));
    var streams = [], i;
    if (contents instanceof PdfStream) streams.push(contents);
    else if (Array.isArray(contents)) {
      for (i = 0; i < contents.length; i++) {
        var s = doc.resolve(contents[i]);
        if (s instanceof PdfStream) streams.push(s);
      }
    }
    if (streams.length === 0) return '';

    var chunks = [], total = 0;
    for (i = 0; i < streams.length; i++) {
      var res = await decodeStream(doc, streams[i], warnings, 'page content');
      if (res.unsupported) {
        warnings.push('A page content stream uses the unsupported /' + res.unsupported +
          ' filter and was skipped.');
        continue;
      }
      if (!res.data) continue;
      chunks.push(res.data, Uint8Array.of(0x0a)); // operators must not run together
      total += res.data.length + 1;
    }
    if (chunks.length === 0) return '';

    var sink = new TextSink();
    var resources = doc.dictOf(page.dict.has('Resources') ? page.dict.get('Resources')
      : page.inherited.Resources);
    await runContent(doc, concatBytes(chunks, total), resources, sink, warnings, fontCache,
      IDENTITY.slice(), 0);
    return sink.toText();
  }

  /* ==================== post-processing and quality ==================== */

  function tidyText(raw) {
    return raw.replace(/\r\n?/g, '\n')
      .replace(/[ \t]+\n/g, '\n')
      .replace(/\n{3,}/g, '\n\n')
      .replace(/[ \t]{2,}/g, ' ')
      .trim();
  }

  var PLAUSIBLE = /[\p{L}\p{N}\p{P}\p{S}\p{M}\s]/u;

  /**
   * Score the extracted text. Anything that smells like a decoding failure has
   * to be caught here, because downstream this text becomes embeddings.
   */
  function assessText(text) {
    var bad = 0, nonSpace = 0, plausible = 0, total = 0;
    for (var ch of text) {
      total++;
      var cp = ch.codePointAt(0);
      var isBad = cp === 0xfffd ||
        (cp < 0x20 && cp !== 0x09 && cp !== 0x0a && cp !== 0x0d) ||
        (cp >= 0xe000 && cp <= 0xf8ff) ||          // private use area
        (cp >= 0xf0000 && cp <= 0x10fffd);         // supplementary private use
      if (isBad) bad++;
      if (!/\s/.test(ch)) {
        nonSpace++;
        if (!isBad && PLAUSIBLE.test(ch)) plausible++;
      }
    }
    return {
      total: total, bad: bad, nonSpace: nonSpace,
      badFraction: total ? bad / total : 0,
      plausibleRatio: nonSpace ? plausible / nonSpace : 0
    };
  }

  function pct(x) { return (Math.max(0, Math.min(1, x)) * 100).toFixed(1) + '%'; }

  /* ==================== public entry point ==================== */

  async function extractPdfText(buffer, onProgress) {
    var warnings = [];
    var report = typeof onProgress === 'function' ? onProgress : function () {};

    if (typeof DecompressionStream !== 'function') {
      throw new Error('This browser is too old to read PDFs in the page (it lacks ' +
        'DecompressionStream). Please update your browser, or convert the PDF to text or ' +
        'Markdown before uploading it.');
    }

    var bytes;
    if (buffer instanceof Uint8Array) bytes = buffer;
    else if (buffer && buffer.byteLength !== undefined) bytes = new Uint8Array(buffer);
    else throw new Error('No PDF data was provided.');
    if (bytes.length === 0) throw new Error('That file is empty, so there is nothing to read.');

    // %PDF- must appear within the first kilobyte; some files carry a preamble.
    if (toLatin1(bytes.subarray(0, Math.min(bytes.length, 1024))).indexOf('%PDF-') < 0) {
      throw new Error('That file is not a PDF (it has no %PDF- header). ' +
        'Please upload a real PDF file.');
    }

    report(0.02, 'Reading file');
    await yieldToEventLoop();

    var doc = new PdfDoc(bytes, warnings);
    doc.scanObjects();
    report(0.08, 'Indexing objects');
    await yieldToEventLoop();

    doc.scanTrailers();
    if (doc.encrypted) {
      throw new Error('This PDF is password-protected (encrypted), so its text cannot be read. ' +
        'Please unlock it, or re-save it without a password, and try again.');
    }
    if (Object.keys(doc.offsets).length === 0) {
      throw new Error('This PDF appears to be damaged — no readable objects were found in it. ' +
        'Try re-saving or re-exporting the file.');
    }

    report(0.12, 'Expanding object streams');
    await doc.expandObjectStreams(function (f) {
      report(0.12 + 0.08 * f, 'Expanding object streams');
    });

    var pages = collectPages(doc, warnings);
    if (pages.length === 0) {
      throw new Error('No pages could be found in this PDF, so there is nothing to read. ' +
        'The file may be damaged; try re-saving or re-exporting it.');
    }

    report(0.22, 'Reading pages');
    var fontCache = Object.create(null), pageTexts = [];
    for (var i = 0; i < pages.length; i++) {
      var pageText = '';
      try {
        pageText = await extractPageText(doc, pages[i], warnings, fontCache);
      } catch (e) {
        warnings.push('Page ' + (i + 1) + ' could not be read and was skipped.');
      }
      pageTexts.push(tidyText(pageText));
      report(0.22 + 0.72 * ((i + 1) / pages.length),
        'Reading page ' + (i + 1) + ' of ' + pages.length);
      // Yield every page so a several-hundred-page document keeps the tab alive.
      await yieldToEventLoop();
    }

    report(0.96, 'Checking quality');
    var text = tidyText(pageTexts.join('\n\n'));
    var stats = assessText(text);

    if (stats.nonSpace === 0 || (stats.total > 0 && stats.nonSpace / stats.total < 0.02)) {
      throw new Error('No text could be read from this PDF. It looks like a scanned or ' +
        'image-only document with no embedded text layer. Run it through OCR first, or paste ' +
        'the text in directly.');
    }

    var confidence = Math.max(0, Math.min(1, (1 - stats.badFraction) * stats.plausibleRatio));
    if (stats.badFraction > 0.02 || stats.plausibleRatio < 0.6) {
      throw new Error('The text in this PDF could not be decoded reliably (decoding confidence ' +
        pct(confidence) + ', with ' + pct(stats.badFraction) + ' unreadable characters) — this ' +
        'usually means an unusual or non-standard font encoding. Please convert the PDF to text ' +
        'or Markdown first and upload that instead.');
    }
    if (confidence < 0.98) {
      warnings.push('Some characters may not have decoded exactly (decoding confidence ' +
        pct(confidence) + ').');
    }

    report(1, 'Done');
    // Repeated observations (one per font, per page) add nothing for the user.
    return { text: text, pages: pages.length, warnings: Array.from(new Set(warnings)) };
  }

  var target = (typeof window !== 'undefined') ? window
    : (typeof globalThis !== 'undefined' ? globalThis : this);
  target.ragPdf = { extractPdfText: extractPdfText };
})();

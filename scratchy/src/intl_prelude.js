// QY16: a minimal en-US `Intl`. QuickJS ships none. Every constructor
// accepts any well-formed locale and answers as en-US (resolvedOptions()
// .locale === 'en-US'); time zones are all UTC. See TODO.txt QY16a for what
// is NOT supported (other languages, real zones, calendars, ...).
(function (g) {
  if (typeof g.Intl !== 'undefined') return;
  var Intl = {};
  function def(o, name, v) { Object.defineProperty(o, name, { value: v, writable: true, configurable: true, enumerable: false }); }
  function methods(F, m) { for (var k in m) def(F.prototype, k, m[k]); }
  function tag(F, name) { Object.defineProperty(F.prototype, Symbol.toStringTag, { value: name, configurable: true }); def(Intl, F.name, F); }
  function getter(F, name, fn) { Object.defineProperty(F.prototype, name, { get: fn, configurable: true }); }
  function needNew(self, F) { if (!(self instanceof F)) throw new TypeError("Constructor Intl." + F.name + " requires 'new'"); }
  var NBSP = ' ';

  // ---- locales ----
  var TAG = /^[a-z]{2,3}(?:-[a-z]{4})?(?:-(?:[a-z]{2}|\d{3}))?(?:-(?:[a-z0-9]{5,8}|\d[a-z0-9]{3}))*(?:-[a-wyz0-9](?:-[a-z0-9]{2,8})+)*(?:-x(?:-[a-z0-9]{1,8})+)?$/i;
  function canonTag(t) {
    if (typeof t !== 'string' && !(t && typeof t === 'object')) throw new TypeError('Incorrect locale information provided');
    t = String(t);
    if (!TAG.test(t)) throw new RangeError('Incorrect locale information provided');
    var parts = t.split('-'), out = [], single = false;
    for (var i = 0; i < parts.length; i++) {
      var p = parts[i];
      if (i === 0) out.push(p.toLowerCase());
      else if (p.length === 1) { single = true; out.push(p.toLowerCase()); }
      else if (single) out.push(p.toLowerCase());
      else if (p.length === 4 && /^[a-z]+$/i.test(p)) out.push(p[0].toUpperCase() + p.slice(1).toLowerCase());
      else if (p.length === 2 && /^[a-z]+$/i.test(p)) out.push(p.toUpperCase());
      else out.push(p.toLowerCase());
    }
    return out.join('-');
  }
  function canonList(l) {
    if (l === undefined) return [];
    if (l === null) throw new TypeError('Cannot convert null to object');
    if (typeof l === 'string' || l instanceof Locale) return [canonTag(l)];
    var out = [], n = l.length >>> 0;
    for (var i = 0; i < n; i++) { var c = canonTag(l[i]); if (out.indexOf(c) < 0) out.push(c); }
    return out;
  }
  function chk(l) { canonList(l); }
  function pick(o, k, allowed, dflt) {
    var v = o == null ? undefined : o[k];
    if (v === undefined) return dflt;
    v = String(v);
    return allowed.indexOf(v) >= 0 ? v : dflt;
  }
  function pickBool(o, k) { var v = o == null ? undefined : o[k]; return v === undefined ? undefined : !!v; }
  function pickInt(o, k, lo, hi, dflt) {
    var v = o == null ? undefined : o[k];
    if (v === undefined) return dflt;
    v = Number(v);
    if (v !== v) return dflt;
    return Math.max(lo, Math.min(hi, Math.floor(v)));
  }
  function opts(o) { if (o === null) throw new TypeError('Cannot convert null to object'); return o === undefined ? {} : Object(o); }
  function pad(n, w) { n = String(n); while (n.length < w) n = '0' + n; return n; }
  function bound(self, key, fn) { var c = self['__' + key]; if (!c) { c = fn.bind(self); Object.defineProperty(self, '__' + key, { value: c, enumerable: false }); } return c; }
  function joinParts(parts) { var s = ''; for (var i = 0; i < parts.length; i++) s += parts[i].value; return s; }

  // ---- decimal string arithmetic (round on the shortest decimal repr, half up) ----
  function plain(s) { // s: nonnegative number string (maybe with exponent) -> {D, P}: value = 0.D * 10^P
    var m = /^(\d*)(?:\.(\d*))?(?:e([+-]?\d+))?$/i.exec(s), ip = m[1] || '0', fp = m[2] || '', e = m[3] ? +m[3] : 0;
    return norm(ip + fp, ip.length + e);
  }
  function norm(D, P) {
    while (D.length > 1 && D.charAt(0) === '0') { D = D.slice(1); P--; }
    if (/^0*$/.test(D)) return { D: '0', P: 1 };
    return { D: D, P: P };
  }
  function incr(s) {
    var a = s.split(''), i = a.length - 1;
    while (i >= 0) { if (a[i] === '9') { a[i] = '0'; i--; } else { a[i] = String(+a[i] + 1); return a.join(''); } }
    return '1' + a.join('');
  }
  function roundAt(p, k) { // keep k digits after the decimal point (k may be negative)
    var keep = p.P + k;
    if (p.D === '0' || keep >= p.D.length) return p;
    if (keep < 0) return { D: '0', P: 1 };
    var up = p.D.charCodeAt(keep) >= 53, head = p.D.slice(0, keep), P = p.P;
    if (up) { head = incr(head); if (head.length > keep) P++; }
    while (head.length < P) head += '0';
    return norm(head, P);
  }
  function shift(p, n) { return p.D === '0' ? p : { D: p.D, P: p.P + n }; }
  function split(p) { // -> {i, f}
    if (p.D === '0') return { i: '0', f: '' };
    var D = p.D, P = p.P;
    if (P <= 0) return { i: '0', f: new Array(-P + 1).join('0') + D };
    while (D.length < P) D += '0';
    return { i: D.slice(0, P), f: D.slice(P) };
  }
  function mag(p) { return p.D === '0' ? 0 : p.P; }

  // ---- NumberFormat ----
  var CUR = { USD: '$', EUR: '€', GBP: '£', JPY: '¥', INR: '₹', CNY: 'CN¥', KRW: '₩', CAD: 'CA$', AUD: 'A$', MXN: 'MX$', BRL: 'R$', NZD: 'NZ$', HKD: 'HK$', ILS: '₪', VND: '₫', TWD: 'NT$', XAF: 'FCFA' };
  var CURNARROW = { CAD: '$', AUD: '$', MXN: '$', NZD: '$', HKD: '$', TWD: '$', CNY: '¥', BRL: 'R$' };
  var CURNAME = { USD: ['US dollar', 'US dollars'], EUR: ['euro', 'euros'], GBP: ['British pound', 'British pounds'], JPY: ['Japanese yen', 'Japanese yen'], CAD: ['Canadian dollar', 'Canadian dollars'], AUD: ['Australian dollar', 'Australian dollars'], INR: ['Indian rupee', 'Indian rupees'], CNY: ['Chinese yuan', 'Chinese yuan'] };
  var CURDIG = { JPY: 0, KRW: 0, VND: 0, CLP: 0, ISK: 0, UGX: 0, XAF: 0, XOF: 0, BHD: 3, KWD: 3, OMR: 3, JOD: 3, TND: 3 };
  var UNITS = { kilometer: 'km', meter: 'm', centimeter: 'cm', millimeter: 'mm', mile: 'mi', foot: 'ft', inch: 'in', kilogram: 'kg', gram: 'g', pound: 'lb', ounce: 'oz', liter: 'L', milliliter: 'mL', second: 'sec', minute: 'min', hour: 'hr', millisecond: 'ms', day: 'day', week: 'wk', month: 'mth', year: 'yr', byte: 'byte', kilobyte: 'kB', megabyte: 'MB', gigabyte: 'GB', terabyte: 'TB', bit: 'bit', celsius: '°C', fahrenheit: '°F', percent: '%', degree: 'deg', 'kilometer-per-hour': 'km/h', 'mile-per-hour': 'mph', 'meter-per-second': 'm/s' };
  var COMPACT_S = ['', 'K', 'M', 'B', 'T'], COMPACT_L = ['', 'thousand', 'million', 'billion', 'trillion'];

  function NumberFormat(locales, options) {
    if (!(this instanceof NumberFormat)) return new NumberFormat(locales, options);
    chk(locales);
    var o = opts(options), r = this.__o = {};
    r.style = pick(o, 'style', ['decimal', 'percent', 'currency', 'unit'], 'decimal');
    if (r.style === 'currency') {
      if (o.currency === undefined) throw new TypeError('Currency code is required with currency style.');
      var c = String(o.currency);
      if (!/^[a-z]{3}$/i.test(c)) throw new RangeError('Invalid currency code : ' + c);
      r.currency = c.toUpperCase();
      r.currencyDisplay = pick(o, 'currencyDisplay', ['code', 'symbol', 'narrowSymbol', 'name'], 'symbol');
      r.currencySign = pick(o, 'currencySign', ['standard', 'accounting'], 'standard');
    }
    if (r.style === 'unit') {
      if (o.unit === undefined) throw new TypeError('Unit is required with unit style.');
      r.unit = String(o.unit);
      r.unitDisplay = pick(o, 'unitDisplay', ['short', 'narrow', 'long'], 'short');
    }
    r.notation = pick(o, 'notation', ['standard', 'scientific', 'engineering', 'compact'], 'standard');
    r.minInt = pickInt(o, 'minimumIntegerDigits', 1, 21, 1);
    var cd = r.style === 'currency' ? (CURDIG[r.currency] !== undefined ? CURDIG[r.currency] : 2) : 0;
    var dMin = r.style === 'currency' ? cd : 0, dMax = r.style === 'currency' ? cd : r.style === 'percent' ? 0 : 3;
    var hasFD = o.minimumFractionDigits !== undefined || o.maximumFractionDigits !== undefined;
    var hasSD = o.minimumSignificantDigits !== undefined || o.maximumSignificantDigits !== undefined;
    if (hasSD) {
      r.minSD = pickInt(o, 'minimumSignificantDigits', 1, 21, 1);
      r.maxSD = Math.max(r.minSD, pickInt(o, 'maximumSignificantDigits', 1, 21, 21));
    } else if (hasFD || r.notation !== 'compact') {
      var mn = pickInt(o, 'minimumFractionDigits', 0, 100, undefined), mx = pickInt(o, 'maximumFractionDigits', 0, 100, undefined);
      if (mn === undefined) mn = mx === undefined ? dMin : Math.min(dMin, mx);
      if (mx === undefined) mx = Math.max(dMax, mn);
      if (mx < mn) mx = mn;
      r.minFD = mn; r.maxFD = mx;
      if (r.notation === 'scientific' || r.notation === 'engineering') { if (!hasFD) { r.minFD = 0; r.maxFD = 3; } }
    }
    r.compactDisplay = pick(o, 'compactDisplay', ['short', 'long'], 'short');
    var ug = o.useGrouping;
    r.useGrouping = ug === undefined || ug === true || ug === 'true' || ug === 'always' || ug === 'auto' || ug === 'min2' ? (ug === undefined ? (r.notation === 'compact' ? 'min2' : 'auto') : (ug === true || ug === 'true' ? 'always' : String(ug))) : false;
    r.signDisplay = pick(o, 'signDisplay', ['auto', 'never', 'always', 'exceptZero', 'negative'], 'auto');
  }
  function nfDigits(r, p) { // round & split a scaled decimal per the digit options
    var q;
    if (r.maxSD !== undefined) {
      q = p.D === '0' ? p : roundAt(p, r.maxSD - mag(p));
      var s = split(q), f = s.f.replace(/0+$/, ''), sig = s.i === '0' ? (f.replace(/^0+/, '')).length : (s.i.length + f.length);
      while (sig < r.minSD) { f += '0'; sig++; }
      return { i: s.i, f: f };
    }
    if (r.minFD === undefined) { // compact default: 2 significant digits, never fewer integer digits
      q = mag(p) >= 2 ? roundAt(p, 0) : roundAt(p, 2 - mag(p));
      var t = split(q); return { i: t.i, f: t.f.replace(/0+$/, '') };
    }
    q = roundAt(p, r.maxFD);
    var u = split(q), ff = u.f.replace(/0+$/, '');
    while (ff.length < r.minFD) ff += '0';
    return { i: u.i, f: ff };
  }
  function group(i) { return i.replace(/\B(?=(\d{3})+$)/g, ','); }
  function nfParts(r, v) {
    var parts = [], neg = false, special = null, p;
    if (typeof v === 'bigint') { neg = v < 0n; p = plain(String(neg ? -v : v)); }
    else {
      v = Number(v);
      if (v !== v) special = 'nan';
      else { neg = v < 0 || (v === 0 && 1 / v < 0); if (!isFinite(v)) special = 'infinity'; else p = plain(String(Math.abs(v))); }
    }
    var body = [], zero = false;
    if (special) body.push(special === 'nan' ? { type: 'nan', value: 'NaN' } : { type: 'infinity', value: '∞' });
    else {
      if (r.style === 'percent') p = shift(p, 2);
      var d, suffix = null, e = 0, sci = false;
      if (r.notation === 'compact') {
        var m3 = mag(p), idx = Math.min(Math.max(0, Math.floor((m3 - 1) / 3)), 4);
        for (;;) {
          d = nfDigits(r, shift(p, -3 * idx));
          if (d.i.length > 3 && idx < 4) idx++; else break;
        }
        if (idx > 0) suffix = r.compactDisplay === 'long' ? COMPACT_L[idx] : COMPACT_S[idx];
      } else if (r.notation === 'scientific' || r.notation === 'engineering') {
        sci = true;
        e = p.D === '0' ? 0 : mag(p) - 1;
        if (r.notation === 'engineering') e = Math.floor(e / 3) * 3;
        d = nfDigits(r, shift(p, -e));
        if (r.notation === 'scientific' && d.i.length > 1) { e++; d = nfDigits(r, shift(p, -e)); }
      } else d = nfDigits(r, p);
      zero = /^0*$/.test(d.i + d.f);
      var ip = d.i; while (ip.length < r.minInt) ip = '0' + ip;
      if (r.useGrouping !== false && !sci && (r.useGrouping !== 'min2' || ip.length >= 5)) {
        var gs = group(ip).split(',');
        for (var gi = 0; gi < gs.length; gi++) { if (gi) body.push({ type: 'group', value: ',' }); body.push({ type: 'integer', value: gs[gi] }); }
      } else body.push({ type: 'integer', value: ip });
      if (d.f) { body.push({ type: 'decimal', value: '.' }); body.push({ type: 'fraction', value: d.f }); }
      if (sci) {
        body.push({ type: 'exponentSeparator', value: 'E' });
        if (e < 0) body.push({ type: 'exponentMinusSign', value: '-' });
        body.push({ type: 'exponentInteger', value: String(Math.abs(e)) });
      }
      if (suffix) { if (r.compactDisplay === 'long') body.push({ type: 'literal', value: ' ' }); body.push({ type: 'compact', value: suffix }); }
    }
    var sd = r.signDisplay, sign = null;
    if (sd === 'always') sign = neg ? '-' : '+';
    else if (sd === 'auto') sign = neg ? '-' : null;
    else if (sd === 'exceptZero') sign = zero || special === 'nan' ? null : (neg ? '-' : '+');
    else if (sd === 'negative') sign = neg && !zero ? '-' : null;
    if (sd === 'exceptZero' && zero) sign = null;
    var signPart = sign ? { type: sign === '-' ? 'minusSign' : 'plusSign', value: sign } : null;
    var acct = r.style === 'currency' && r.currencySign === 'accounting' && sign === '-';
    if (r.style === 'currency') {
      var disp = r.currencyDisplay, sym;
      if (disp === 'code') sym = r.currency;
      else if (disp === 'name') sym = null;
      else sym = (disp === 'narrowSymbol' && CURNARROW[r.currency]) || CUR[r.currency] || r.currency;
      if (sym === null) {
        var nm = CURNAME[r.currency] || [r.currency, r.currency], one = body.length === 1 && body[0].value === '1';
        if (signPart) parts.push(signPart);
        parts = parts.concat(body); parts.push({ type: 'literal', value: ' ' }); parts.push({ type: 'currency', value: one ? nm[0] : nm[1] });
      } else {
        if (acct) parts.push({ type: 'literal', value: '(' }); else if (signPart) parts.push(signPart);
        parts.push({ type: 'currency', value: sym });
        if (/^[A-Za-z]/.test(sym.charAt(sym.length - 1))) parts.push({ type: 'literal', value: NBSP });
        parts = parts.concat(body);
        if (acct) parts.push({ type: 'literal', value: ')' });
      }
    } else {
      if (signPart) parts.push(signPart);
      parts = parts.concat(body);
      if (r.style === 'percent') parts.push({ type: 'percentSign', value: '%' });
      else if (r.style === 'unit') {
        var us = UNITS[r.unit] || r.unit, plural = !(body.length === 1 && body[0].value === '1');
        if (r.unitDisplay === 'long') { parts.push({ type: 'literal', value: ' ' }); parts.push({ type: 'unit', value: r.unit.replace(/-per-/, ' per ') + (plural && !/per/.test(r.unit) ? 's' : '') }); }
        else if (r.unitDisplay === 'narrow' || /^[°%]/.test(us)) parts.push({ type: 'unit', value: us });
        else { parts.push({ type: 'literal', value: ' ' }); parts.push({ type: 'unit', value: us + (us === 'day' && plural ? 's' : '') }); }
      }
    }
    return parts;
  }
  function numArg(x) { return typeof x === 'bigint' ? x : (x !== null && typeof x === 'object' && typeof x.valueOf === 'function' ? Number(x) : Number(x)); }
  methods(NumberFormat, {
    formatToParts: function formatToParts(x) { return nfParts(this.__o, numArg(x)); },
    formatRange: function formatRange(a, b) { return nfParts(this.__o, numArg(a)).map(function (p) { return p.value; }).join('') + '–' + joinParts(nfParts(this.__o, numArg(b))); },
    resolvedOptions: function resolvedOptions() {
      var r = this.__o, o = { locale: 'en-US', numberingSystem: 'latn', style: r.style };
      if (r.currency) { o.currency = r.currency; o.currencyDisplay = r.currencyDisplay; o.currencySign = r.currencySign; }
      if (r.unit) { o.unit = r.unit; o.unitDisplay = r.unitDisplay; }
      o.minimumIntegerDigits = r.minInt;
      if (r.minFD !== undefined) { o.minimumFractionDigits = r.minFD; o.maximumFractionDigits = r.maxFD; }
      if (r.maxSD !== undefined) { o.minimumSignificantDigits = r.minSD; o.maximumSignificantDigits = r.maxSD; }
      o.useGrouping = r.useGrouping; o.notation = r.notation;
      if (r.notation === 'compact') o.compactDisplay = r.compactDisplay;
      o.signDisplay = r.signDisplay; o.roundingIncrement = 1; o.roundingMode = 'halfExpand'; o.roundingPriority = 'auto'; o.trailingZeroDisplay = 'auto';
      return o;
    }
  });
  getter(NumberFormat, 'format', function () { return bound(this, 'format', function (x) { return joinParts(nfParts(this.__o, numArg(x))); }); });
  tag(NumberFormat, 'Intl.NumberFormat');

  // ---- DateTimeFormat ----
  var MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
  var DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
  var DATE_KEYS = ['weekday', 'year', 'month', 'day'], TIME_KEYS = ['dayPeriod', 'hour', 'minute', 'second', 'fractionalSecondDigits'];
  function DateTimeFormat(locales, options, required, defaults) {
    if (!(this instanceof DateTimeFormat)) return new DateTimeFormat(locales, options);
    chk(locales);
    var o = opts(options), r = this.__o = {}, i;
    required = required || 'any'; defaults = defaults || 'date';
    r.weekday = pick(o, 'weekday', ['narrow', 'short', 'long']);
    r.year = pick(o, 'year', ['2-digit', 'numeric']);
    r.month = pick(o, 'month', ['2-digit', 'numeric', 'narrow', 'short', 'long']);
    r.day = pick(o, 'day', ['2-digit', 'numeric']);
    r.hour = pick(o, 'hour', ['2-digit', 'numeric']);
    r.minute = pick(o, 'minute', ['2-digit', 'numeric']);
    r.second = pick(o, 'second', ['2-digit', 'numeric']);
    r.fsd = pickInt(o, 'fractionalSecondDigits', 1, 3, undefined);
    r.dayPeriod = pick(o, 'dayPeriod', ['narrow', 'short', 'long']);
    r.tzName = pick(o, 'timeZoneName', ['short', 'long', 'shortOffset', 'longOffset', 'shortGeneric', 'longGeneric']);
    r.dateStyle = pick(o, 'dateStyle', ['full', 'long', 'medium', 'short']);
    r.timeStyle = pick(o, 'timeStyle', ['full', 'long', 'medium', 'short']);
    if (o.timeZone !== undefined) {
      var tz = String(o.timeZone);
      if (!/^[A-Za-z][A-Za-z0-9_\/+\-]*$|^[+-]\d\d(:?\d\d)?$/.test(tz)) throw new RangeError('Invalid time zone specified: ' + tz);
    }
    var h12 = pickBool(o, 'hour12'), hc = pick(o, 'hourCycle', ['h11', 'h12', 'h23', 'h24']);
    r.hour12 = h12 !== undefined ? h12 : hc ? (hc === 'h11' || hc === 'h12') : true;
    r.hc = r.hour12 ? 'h12' : 'h23';
    if (r.dateStyle || r.timeStyle) {
      var ds = r.dateStyle, ts = r.timeStyle;
      if (ds) {
        r.weekday = ds === 'full' ? 'long' : undefined;
        r.month = ds === 'full' || ds === 'long' ? 'long' : ds === 'medium' ? 'short' : 'numeric';
        r.day = 'numeric'; r.year = ds === 'short' ? '2-digit' : 'numeric';
      }
      if (ts) { r.hour = 'numeric'; r.minute = '2-digit'; r.second = ts === 'short' ? undefined : '2-digit'; r.tzName = ts === 'full' ? 'long' : ts === 'long' ? 'short' : undefined; }
    } else {
      var need = true;
      if (required !== 'time') for (i = 0; i < DATE_KEYS.length; i++) if (r[DATE_KEYS[i]]) need = false;
      if (required !== 'date') for (i = 0; i < TIME_KEYS.length; i++) if (r[TIME_KEYS[i]] !== undefined) need = false;
      if (need) {
        if (defaults !== 'time') { r.year = 'numeric'; r.month = 'numeric'; r.day = 'numeric'; }
        if (defaults !== 'date') { r.hour = 'numeric'; r.minute = 'numeric'; r.second = 'numeric'; }
      }
    }
  }
  function dtTime(x) {
    var t = x === undefined ? Date.now() : (x instanceof Date ? x.getTime() : Number(x));
    if (t !== t || Math.abs(t) > 8.64e15) throw new RangeError('Invalid time value');
    return t;
  }
  function dtParts(r, t) {
    var d = new Date(t), out = [], lit = function (s) { out.push({ type: 'literal', value: s }); }, put = function (ty, v) { out.push({ type: ty, value: String(v) }); };
    var wd = r.weekday && DAYS[d.getUTCDay()], hasDate = r.weekday || r.year || r.month || r.day;
    var y = d.getUTCFullYear(), yv = r.year === '2-digit' ? pad(((y % 100) + 100) % 100, 2) : String(y);
    var textual = r.month === 'long' || r.month === 'short' || r.month === 'narrow';
    function wdp() { put('weekday', r.weekday === 'long' ? wd : r.weekday === 'short' ? wd.slice(0, 3) : wd[0]); }
    if (hasDate) {
      if (textual) {
        if (r.weekday) { wdp(); lit(', '); }
        var mn = MONTHS[d.getUTCMonth()];
        put('month', r.month === 'long' ? mn : r.month === 'short' ? mn.slice(0, 3) : mn[0]);
        if (r.day) { lit(' '); put('day', r.day === '2-digit' ? pad(d.getUTCDate(), 2) : d.getUTCDate()); }
        if (r.year) { lit(r.day ? ', ' : ' '); put('year', yv); }
      } else {
        if (r.weekday) { wdp(); if (r.month || r.day || r.year) lit(', '); }
        var seq = [];
        if (r.month) seq.push(['month', r.month === '2-digit' ? pad(d.getUTCMonth() + 1, 2) : d.getUTCMonth() + 1]);
        if (r.day) seq.push(['day', r.day === '2-digit' ? pad(d.getUTCDate(), 2) : d.getUTCDate()]);
        if (r.year) seq.push(['year', yv]);
        for (var i = 0; i < seq.length; i++) { if (i) lit('/'); put(seq[i][0], seq[i][1]); }
      }
    }
    var hasTime = r.hour || r.minute || r.second || r.fsd;
    if (hasTime) {
      if (hasDate) lit(textual && r.month === 'long' ? ' at ' : ', ');
      var H = d.getUTCHours(), n = 0;
      if (r.hour) {
        var hv = r.hour12 ? (H % 12 || 12) : H;
        put('hour', r.hour === '2-digit' || (!r.hour12 && (r.minute || r.second)) ? pad(hv, 2) : hv); n++;
      }
      if (r.minute) { if (n) lit(':'); put('minute', n || r.minute === '2-digit' ? pad(d.getUTCMinutes(), 2) : d.getUTCMinutes()); n++; }
      if (r.second) { if (n) lit(':'); put('second', n || r.second === '2-digit' ? pad(d.getUTCSeconds(), 2) : d.getUTCSeconds()); n++; }
      if (r.fsd) { if (n) lit('.'); put('fractionalSecond', pad(d.getUTCMilliseconds(), 3).slice(0, r.fsd)); }
      if (r.hour && r.hour12) { lit(' '); put('dayPeriod', H < 12 ? 'AM' : 'PM'); }
    }
    if (r.tzName) {
      if (hasDate || hasTime) lit(' ');
      put('timeZoneName', r.tzName === 'long' ? 'Coordinated Universal Time' : r.tzName === 'shortOffset' ? 'GMT' : r.tzName === 'longOffset' ? 'GMT' : 'UTC');
    }
    return out;
  }
  methods(DateTimeFormat, {
    formatToParts: function formatToParts(x) { return dtParts(this.__o, dtTime(x)); },
    formatRange: function formatRange(a, b) { return joinParts(dtParts(this.__o, dtTime(a))) + ' – ' + joinParts(dtParts(this.__o, dtTime(b))); },
    formatRangeToParts: function formatRangeToParts(a, b) { return dtParts(this.__o, dtTime(a)).concat([{ type: 'literal', value: ' – ' }], dtParts(this.__o, dtTime(b))); },
    resolvedOptions: function resolvedOptions() {
      var r = this.__o, o = { locale: 'en-US', calendar: 'gregory', numberingSystem: 'latn', timeZone: 'UTC' };
      if (r.hour) { o.hourCycle = r.hc; o.hour12 = r.hour12; }
      var ks = ['weekday', 'year', 'month', 'day', 'dayPeriod', 'hour', 'minute', 'second', 'fsd', 'tzName', 'dateStyle', 'timeStyle'];
      for (var i = 0; i < ks.length; i++) if (r[ks[i]] !== undefined && !(r.dateStyle || r.timeStyle) || (r[ks[i]] !== undefined && /Style$/.test(ks[i]))) o[ks[i] === 'fsd' ? 'fractionalSecondDigits' : ks[i] === 'tzName' ? 'timeZoneName' : ks[i]] = r[ks[i]];
      return o;
    }
  });
  getter(DateTimeFormat, 'format', function () { return bound(this, 'format', function (x) { return joinParts(dtParts(this.__o, dtTime(x))); }); });
  tag(DateTimeFormat, 'Intl.DateTimeFormat');

  // ---- PluralRules ----
  function PluralRules(locales, options) {
    needNew(this, PluralRules); chk(locales);
    this.__t = pick(opts(options), 'type', ['cardinal', 'ordinal'], 'cardinal');
  }
  methods(PluralRules, {
    select: function select(n) {
      n = Number(n);
      if (this.__t === 'ordinal') {
        var a = Math.abs(n);
        if (a !== Math.floor(a)) return 'other';
        var m10 = a % 10, m100 = a % 100;
        return m10 === 1 && m100 !== 11 ? 'one' : m10 === 2 && m100 !== 12 ? 'two' : m10 === 3 && m100 !== 13 ? 'few' : 'other';
      }
      return Math.abs(n) === 1 ? 'one' : 'other';
    },
    selectRange: function selectRange(a, b) { return this.select(b); },
    resolvedOptions: function resolvedOptions() {
      return { locale: 'en-US', type: this.__t, minimumIntegerDigits: 1, minimumFractionDigits: 0, maximumFractionDigits: 3, pluralCategories: this.__t === 'ordinal' ? ['few', 'one', 'two', 'other'] : ['one', 'other'] };
    }
  });
  tag(PluralRules, 'Intl.PluralRules');
  PluralRules.supportedLocalesOf = supportedLocalesOf;

  // ---- RelativeTimeFormat ----
  var RT_UNITS = { second: ['sec.', 'now'], minute: ['min.', 'this minute'], hour: ['hr.', 'this hour'], day: ['day', null], week: ['wk.', null], month: ['mo.', null], quarter: ['qtr.', null], year: ['yr.', null] };
  var RT_SPECIAL = { day: ['yesterday', 'today', 'tomorrow'], week: ['last week', 'this week', 'next week'], month: ['last month', 'this month', 'next month'], quarter: ['last quarter', 'this quarter', 'next quarter'], year: ['last year', 'this year', 'next year'], hour: [null, 'this hour', null], minute: [null, 'this minute', null], second: [null, 'now', null] };
  function RelativeTimeFormat(locales, options) {
    needNew(this, RelativeTimeFormat); chk(locales);
    var o = opts(options);
    this.__s = pick(o, 'style', ['long', 'short', 'narrow'], 'long');
    this.__n = pick(o, 'numeric', ['always', 'auto'], 'always');
    this.__nf = new NumberFormat('en-US');
  }
  function rtParts(self, value, unit) {
    value = Number(value);
    if (!isFinite(value)) throw new RangeError('Value need to be finite number for Intl.RelativeTimeFormat.prototype.format()');
    unit = String(unit);
    var u = unit.replace(/s$/, '');
    if (!RT_UNITS[u]) throw new RangeError('Invalid unit argument for format() \'' + unit + '\'');
    if (self.__n === 'auto' && RT_SPECIAL[u] && (value === 0 || value === 1 || value === -1)) {
      var sp = RT_SPECIAL[u][value + 1];
      if (sp) return [{ type: 'literal', value: sp }];
    }
    var neg = value < 0 || (value === 0 && 1 / value < 0), a = Math.abs(value), np = self.__nf.formatToParts(a), plural = a !== 1;
    var name;
    if (self.__s === 'long') name = u + (plural ? 's' : '');
    else name = u === 'day' ? (plural ? 'days' : 'day') : u === 'quarter' ? (plural ? 'qtrs.' : 'qtr.') : RT_UNITS[u][0];
    var out = [];
    if (!neg) out.push({ type: 'literal', value: 'in ' });
    for (var i = 0; i < np.length; i++) out.push({ type: np[i].type, value: np[i].value, unit: u });
    out.push({ type: 'literal', value: ' ' + name + (neg ? ' ago' : '') });
    return out;
  }
  methods(RelativeTimeFormat, {
    format: function format(v, u) { return joinParts(rtParts(this, v, u)); },
    formatToParts: function formatToParts(v, u) { return rtParts(this, v, u); },
    resolvedOptions: function resolvedOptions() { return { locale: 'en-US', style: this.__s, numeric: this.__n, numberingSystem: 'latn' }; }
  });
  tag(RelativeTimeFormat, 'Intl.RelativeTimeFormat');
  RelativeTimeFormat.supportedLocalesOf = supportedLocalesOf;

  // ---- ListFormat ----
  function ListFormat(locales, options) {
    needNew(this, ListFormat); chk(locales);
    var o = opts(options);
    this.__t = pick(o, 'type', ['conjunction', 'disjunction', 'unit'], 'conjunction');
    this.__s = pick(o, 'style', ['long', 'short', 'narrow'], 'long');
  }
  function lfParts(self, list) {
    var items = [];
    if (list !== undefined) for (var v of list) { if (typeof v !== 'string') throw new TypeError('Iterable yielded ' + String(v) + ' which is not a string'); items.push(v); }
    var t = self.__t, s = self.__s, word, pair, last, mid = ', ';
    if (t === 'conjunction') { word = s === 'long' ? 'and' : s === 'short' ? '&' : null; }
    else if (t === 'disjunction') word = 'or';
    else word = null;
    if (word) { pair = ' ' + word + ' '; last = s === 'long' || t === 'disjunction' ? ', ' + word + ' ' : ', ' + word + ' '; }
    else { mid = t === 'unit' && s === 'narrow' ? ' ' : ', '; pair = mid; last = mid; }
    var out = [];
    for (var i = 0; i < items.length; i++) {
      if (i) out.push({ type: 'literal', value: items.length === 2 ? pair : i === items.length - 1 ? last : mid });
      out.push({ type: 'element', value: items[i] });
    }
    return out;
  }
  methods(ListFormat, {
    format: function format(l) { return joinParts(lfParts(this, l)); },
    formatToParts: function formatToParts(l) { return lfParts(this, l); },
    resolvedOptions: function resolvedOptions() { return { locale: 'en-US', type: this.__t, style: this.__s }; }
  });
  tag(ListFormat, 'Intl.ListFormat');
  ListFormat.supportedLocalesOf = supportedLocalesOf;

  // ---- Collator (root-ish collation: punctuation < digits < letters, case-insensitive first) ----
  function Collator(locales, options) {
    if (!(this instanceof Collator)) return new Collator(locales, options);
    chk(locales);
    var o = opts(options);
    this.__o = { usage: pick(o, 'usage', ['sort', 'search'], 'sort'), sensitivity: pick(o, 'sensitivity', ['base', 'accent', 'case', 'variant'], 'variant'), numeric: !!o.numeric, caseFirst: pick(o, 'caseFirst', ['upper', 'lower', 'false'], 'false'), ignorePunctuation: !!o.ignorePunctuation };
  }
  function baseOf(s) { return s.normalize('NFD').replace(/[̀-ͯ]/g, ''); }
  function cls(c) { return /[\p{L}]/u.test(c) ? 2 : /[\p{N}]/u.test(c) ? 1 : 0; }
  function primary(a, b, numeric) {
    var i = 0, j = 0;
    while (i < a.length && j < b.length) {
      var ca = a.charAt(i), cb = b.charAt(j);
      if (numeric && /\d/.test(ca) && /\d/.test(cb)) {
        var ma = /^\d+/.exec(a.slice(i))[0], mb = /^\d+/.exec(b.slice(j))[0];
        var na = ma.replace(/^0+(?=\d)/, ''), nb = mb.replace(/^0+(?=\d)/, '');
        if (na.length !== nb.length) return na.length < nb.length ? -1 : 1;
        if (na !== nb) return na < nb ? -1 : 1;
        i += ma.length; j += mb.length; continue;
      }
      if (ca !== cb) {
        var ka = cls(ca), kb = cls(cb);
        if (ka !== kb) return ka < kb ? -1 : 1;
        return ca < cb ? -1 : 1;
      }
      i++; j++;
    }
    return a.length - i === b.length - j ? 0 : (a.length - i) < (b.length - j) ? -1 : 1;
  }
  function collate(o, a, b) {
    a = String(a); b = String(b);
    if (o.ignorePunctuation) { a = a.replace(/[\p{P}\s]/gu, ''); b = b.replace(/[\p{P}\s]/gu, ''); }
    var ba = baseOf(a).toLowerCase(), bb = baseOf(b).toLowerCase(), r = primary(ba, bb, o.numeric);
    if (r || o.sensitivity === 'base') return r;
    if (o.sensitivity !== 'case') {
      var aa = a.normalize('NFD').toLowerCase(), ab = b.normalize('NFD').toLowerCase();
      if (aa !== ab) {
        var ca = aa.match(/[^\u0300-\u036f][\u0300-\u036f]*/g) || [], cb = ab.match(/[^\u0300-\u036f][\u0300-\u036f]*/g) || [];
        for (var q = 0; q < ca.length && q < cb.length; q++) if (ca[q] !== cb[q]) { var ma = ca[q].slice(1), mb = cb[q].slice(1); return ma === '' ? -1 : mb === '' ? 1 : ma < mb ? -1 : 1; }
      }
    }
    if (o.sensitivity === 'accent') return 0;
    var la = baseOf(a), lb = baseOf(b);
    if (la !== lb) {
      for (var i = 0; i < la.length && i < lb.length; i++) if (la.charAt(i) !== lb.charAt(i)) { var up = la.charAt(i) === la.charAt(i).toUpperCase(); return (up ? 1 : -1) * (o.caseFirst === 'upper' ? -1 : 1); }
    }
    return 0;
  }
  methods(Collator, {
    resolvedOptions: function resolvedOptions() { var o = this.__o; return { locale: 'en-US', usage: o.usage, sensitivity: o.sensitivity, ignorePunctuation: o.ignorePunctuation, collation: 'default', numeric: o.numeric, caseFirst: o.caseFirst }; }
  });
  getter(Collator, 'compare', function () { return bound(this, 'compare', function (a, b) { return collate(this.__o, a, b); }); });
  tag(Collator, 'Intl.Collator');
  Collator.supportedLocalesOf = supportedLocalesOf;
  var defCollator = null;
  function localeCompare(that, locales, options) {
    if (this == null) throw new TypeError('String.prototype.localeCompare called on null or undefined');
    var c;
    if (locales === undefined && options === undefined) c = defCollator || (defCollator = new Collator());
    else c = new Collator(locales, options);
    return c.compare(String(this), String(that));
  }

  // ---- Segmenter ----
  function Segmenter(locales, options) {
    needNew(this, Segmenter); chk(locales);
    this.__g = pick(opts(options), 'granularity', ['grapheme', 'word', 'sentence'], 'grapheme');
  }
  var RE_G = /\p{RI}\p{RI}|(?:\P{M}|^)\p{M}*(?:‍\p{Extended_Pictographic}\p{M}*|[️\u{1f3fb}-\u{1f3ff}])*|[\s\S]/gu;
  function segs(g, s) {
    var out = [], m, re;
    if (g === 'word') re = /[\p{L}\p{N}_]+(?:['’.][\p{L}\p{N}_]+)*|\s+|[\s\S]/gu;
    else if (g === 'sentence') re = /[^.!?]+(?:[.!?]+\s*|$)|[.!?]+\s*/g;
    else re = /\p{RI}\p{RI}|[\s\S][\p{M}️‍\u{1f3fb}-\u{1f3ff}]*(?:‍[\s\S][\p{M}️\u{1f3fb}-\u{1f3ff}]*)*/gu;
    while ((m = re.exec(s)) !== null) {
      if (!m[0].length) { re.lastIndex++; continue; }
      var o = { segment: m[0], index: m.index, input: s };
      if (g === 'word') o.isWordLike = /[\p{L}\p{N}]/u.test(m[0]);
      out.push(o);
    }
    return out;
  }
  methods(Segmenter, {
    segment: function segment(str) {
      var list = segs(this.__g, String(str));
      return {
        containing: function containing(i) { i = i === undefined ? 0 : Math.floor(Number(i)) || 0; for (var k = 0; k < list.length; k++) if (i >= list[k].index && i < list[k].index + list[k].segment.length) return list[k]; return undefined; },
        [Symbol.iterator]: function () { return list[Symbol.iterator](); }
      };
    },
    resolvedOptions: function resolvedOptions() { return { locale: 'en-US', granularity: this.__g }; }
  });
  tag(Segmenter, 'Intl.Segmenter');
  Segmenter.supportedLocalesOf = supportedLocalesOf;

  // ---- DisplayNames (stub: the code itself) ----
  function DisplayNames(locales, options) {
    needNew(this, DisplayNames); chk(locales);
    var o = opts(options);
    if (o.type === undefined) throw new TypeError('Required option "type" is missing');
    this.__t = pick(o, 'type', ['language', 'region', 'script', 'currency', 'calendar', 'dateTimeField'], 'language');
    this.__f = pick(o, 'fallback', ['code', 'none'], 'code');
    this.__s = pick(o, 'style', ['narrow', 'short', 'long'], 'long');
  }
  methods(DisplayNames, {
    of: function of(code) { code = String(code); return this.__f === 'none' ? undefined : code; },
    resolvedOptions: function resolvedOptions() { return { locale: 'en-US', style: this.__s, type: this.__t, fallback: this.__f }; }
  });
  tag(DisplayNames, 'Intl.DisplayNames');
  DisplayNames.supportedLocalesOf = supportedLocalesOf;

  // ---- Locale ----
  var LIKELY = { en: ['Latn', 'US'], fr: ['Latn', 'FR'], de: ['Latn', 'DE'], es: ['Latn', 'ES'], it: ['Latn', 'IT'], pt: ['Latn', 'BR'], nl: ['Latn', 'NL'], sv: ['Latn', 'SE'], pl: ['Latn', 'PL'], tr: ['Latn', 'TR'], ru: ['Cyrl', 'RU'], uk: ['Cyrl', 'UA'], ja: ['Jpan', 'JP'], zh: ['Hans', 'CN'], ko: ['Kore', 'KR'], ar: ['Arab', 'EG'], he: ['Hebr', 'IL'], hi: ['Deva', 'IN'], el: ['Grek', 'GR'], und: ['Latn', 'US'] };
  function Locale(tagArg, options) {
    needNew(this, Locale);
    if (typeof tagArg !== 'string' && !(tagArg instanceof Locale)) throw new TypeError('First argument to Intl.Locale constructor can\'t be empty or missing');
    var t = canonTag(tagArg instanceof Locale ? tagArg.toString() : tagArg), o = opts(options);
    var segs = t.split('-'), ext = '', xi = -1, i;
    for (i = 1; i < segs.length; i++) if (segs[i].length === 1) { xi = i; break; }
    if (xi >= 0) { ext = segs.slice(xi).join('-'); segs = segs.slice(0, xi); }
    var lang = segs[0], script, region, variants = [];
    for (i = 1; i < segs.length; i++) {
      if (!script && !region && !variants.length && segs[i].length === 4 && /^[A-Za-z]+$/.test(segs[i])) script = segs[i];
      else if (!region && !variants.length && (segs[i].length === 2 || /^\d{3}$/.test(segs[i]))) region = segs[i];
      else variants.push(segs[i]);
    }
    if (o.language !== undefined) lang = canonTag(String(o.language)).toLowerCase();
    if (o.script !== undefined) script = canonTag(lang + '-' + o.script).split('-')[1];
    if (o.region !== undefined) region = String(o.region).toUpperCase();
    var kv = {}, um = /(?:^|-)u((?:-[a-z0-9]{2,8})+)/.exec(ext);
    if (um) { var ps = um[1].slice(1).split('-'), key = null; for (i = 0; i < ps.length; i++) { if (ps[i].length === 2) { key = ps[i]; kv[key] = ''; } else if (key) kv[key] = kv[key] ? kv[key] + '-' + ps[i] : ps[i]; } }
    var map = { calendar: 'ca', collation: 'co', hourCycle: 'hc', caseFirst: 'kf', numeric: 'kn', numberingSystem: 'nu' };
    for (var k in map) if (o[k] !== undefined) kv[map[k]] = k === 'numeric' ? (o[k] ? '' : 'false') : String(o[k]);
    var d = { language: lang, script: script, region: region, variants: variants, kv: kv, ext: um ? ext.replace(/(?:^|-)u(?:-[a-z0-9]{2,8})+/, '').replace(/^-/, '') : ext };
    Object.defineProperty(this, '__l', { value: d });
  }
  function locBase(d) { return [d.language, d.script, d.region].concat(d.variants).filter(Boolean).join('-'); }
  function locStr(d) {
    var s = locBase(d), keys = Object.keys(d.kv).sort();
    if (keys.length) s += '-u' + keys.map(function (k) { return '-' + k + (d.kv[k] === '' ? '' : '-' + d.kv[k]); }).join('');
    return d.ext ? s + '-' + d.ext : s;
  }
  function fromD(d) { return new Locale(locStr(d)); }
  methods(Locale, {
    toString: function toString() { return locStr(this.__l); },
    maximize: function maximize() { var d = this.__l, l = LIKELY[d.language] || LIKELY.und, n = {}; for (var k in d) n[k] = d[k]; n.script = d.script || l[0]; n.region = d.region || l[1]; return fromD(n); },
    minimize: function minimize() { var d = this.__l, l = LIKELY[d.language], n = {}; for (var k in d) n[k] = d[k]; if (l) { if (d.script === l[0]) n.script = undefined; if (d.region === l[1]) n.region = undefined; } return fromD(n); }
  });
  var LG = { language: function (d) { return d.language; }, script: function (d) { return d.script; }, region: function (d) { return d.region; }, baseName: locBase, calendar: function (d) { return d.kv.ca; }, collation: function (d) { return d.kv.co; }, hourCycle: function (d) { return d.kv.hc; }, caseFirst: function (d) { return d.kv.kf; }, numberingSystem: function (d) { return d.kv.nu; }, numeric: function (d) { return d.kv.kn === '' || d.kv.kn === 'true'; } };
  Object.keys(LG).forEach(function (k) { getter(Locale, k, function () { return LG[k](this.__l); }); });
  tag(Locale, 'Intl.Locale');

  // ---- Intl namespace functions ----
  function supportedLocalesOf(locales) { return canonList(locales).filter(function (t) { return /^en(-|$)/.test(t); }); }
  [NumberFormat, DateTimeFormat].forEach(function (F) { F.supportedLocalesOf = supportedLocalesOf; });
  def(Intl, 'getCanonicalLocales', function getCanonicalLocales(l) { return canonList(l); });
  def(Intl, 'supportedValuesOf', function supportedValuesOf(key) {
    key = String(key);
    if (['calendar', 'collation', 'currency', 'numberingSystem', 'timeZone', 'unit'].indexOf(key) < 0) throw new RangeError('Invalid key : ' + key);
    return key === 'timeZone' ? ['UTC'] : [];
  });
  Object.defineProperty(Intl, Symbol.toStringTag, { value: 'Intl', configurable: true });
  def(g, 'Intl', Intl);

  // ---- toLocale*String delegation ----
  function nfOf(l, o) { return new NumberFormat(l, o); }
  def(Number.prototype, 'toLocaleString', function toLocaleString(l, o) { return nfOf(l, o).format(Number.prototype.valueOf.call(this)); });
  if (typeof BigInt === 'function') def(BigInt.prototype, 'toLocaleString', function toLocaleString(l, o) { return nfOf(l, o).format(BigInt.prototype.valueOf.call(this)); });
  function dateLocale(name, required, defaults) {
    def(Date.prototype, name, function (l, o) {
      var t = Date.prototype.getTime.call(this);
      if (t !== t) return 'Invalid Date';
      return new DateTimeFormat(l, o, required, defaults).format(t);
    });
  }
  dateLocale('toLocaleString', 'any', 'all');
  dateLocale('toLocaleDateString', 'date', 'date');
  dateLocale('toLocaleTimeString', 'time', 'time');
  def(String.prototype, 'localeCompare', localeCompare);
  def(Array.prototype, 'toLocaleString', function toLocaleString(l, o) {
    var out = [];
    for (var i = 0; i < this.length; i++) out.push(this[i] == null ? '' : this[i].toLocaleString(l, o));
    return out.join(',');
  });
})(globalThis);

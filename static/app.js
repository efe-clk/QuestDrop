/* QuestDrop progressive enhancements. Everything here is optional: every
   form and page works without it (docs/web-ui.md §2). Each initializer is
   idempotent and runs on first load and on every HTMX swap. */
(function () {
  "use strict";

  function store(kind) {
    try { return kind === "local" ? window.localStorage : window.sessionStorage; } catch (e) { return null; }
  }
  function get(kind, key) {
    var s = store(kind);
    if (!s) return null;
    try { return s.getItem(key); } catch (e) { return null; }
  }
  function set(kind, key, val) {
    var s = store(kind);
    if (!s) return;
    try { s.setItem(key, val); } catch (e) { /* quota or privacy mode */ }
  }
  function del(kind, key) {
    var s = store(kind);
    if (!s) return;
    try { s.removeItem(key); } catch (e) { /* ignore */ }
  }
  function once(el, flag) {
    if (el.dataset[flag]) return false;
    el.dataset[flag] = "1";
    return true;
  }
  function each(root, sel, fn) {
    var list = [];
    if (root.matches && root.matches(sel)) list.push(root);
    root.querySelectorAll && root.querySelectorAll(sel).forEach(function (e) { list.push(e); });
    list.forEach(fn);
  }
  window.QD = window.QD || {};
  window.QD.store = { get: get, set: set, del: del };

  /* ---- chip input: "rust, go" text field -> removable chips ---- */
  function chips(input) {
    if (!once(input, "chipsReady")) return;
    var max = parseInt(input.dataset.max || "10", 10);
    var wrap = document.createElement("div");
    wrap.className = "chipin";
    var text = document.createElement("input");
    text.type = "text";
    text.className = "chipin__text";
    text.id = input.id;
    text.setAttribute("autocomplete", "off");
    text.setAttribute("autocapitalize", "none");
    if (input.getAttribute("aria-describedby")) text.setAttribute("aria-describedby", input.getAttribute("aria-describedby"));
    input.removeAttribute("id");
    input.type = "hidden";
    input.parentNode.insertBefore(wrap, input);
    var list = [];
    function sync() {
      input.value = list.join(", ");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }
    function render() {
      wrap.querySelectorAll(".chipin__chip").forEach(function (c) { c.remove(); });
      list.forEach(function (v, i) {
        var b = document.createElement("button");
        b.type = "button";
        b.className = "chipin__chip";
        b.textContent = v;
        b.setAttribute("aria-label", v + " ×");
        b.addEventListener("click", function () {
          list.splice(i, 1); render(); sync(); text.focus();
        });
        wrap.insertBefore(b, text);
      });
      text.placeholder = list.length >= max ? "" : "+";
      text.disabled = list.length >= max;
    }
    function add(raw) {
      raw.split(",").forEach(function (part) {
        var v = part.trim().toLowerCase().slice(0, 32);
        if (v && list.indexOf(v) < 0 && list.length < max) list.push(v);
      });
    }
    add(input.value || "");
    wrap.appendChild(text);
    text.addEventListener("keydown", function (e) {
      if (e.key === "Enter" || e.key === ",") {
        if (text.value.trim()) { e.preventDefault(); add(text.value); text.value = ""; render(); sync(); }
        else if (e.key === ",") e.preventDefault();
      } else if (e.key === "Backspace" && !text.value && list.length) {
        list.pop(); render(); sync();
      }
    });
    text.addEventListener("blur", function () {
      if (text.value.trim()) { add(text.value); text.value = ""; render(); sync(); }
    });
    text.addEventListener("paste", function () {
      setTimeout(function () { if (text.value.indexOf(",") >= 0) { add(text.value); text.value = ""; render(); sync(); } }, 0);
    });
    wrap.addEventListener("click", function (e) { if (e.target === wrap) text.focus(); });
    // A pending word must not be lost when the form is submitted.
    var form = input.form;
    if (form) form.addEventListener("submit", function () {
      if (text.value.trim()) { add(text.value); text.value = ""; render(); sync(); }
    }, true);
    render();
  }

  /* ---- first-2-minutes checklist, kept only in this browser ---- */
  function revive(box) {
    if (!once(box, "reviveReady")) return;
    var key = box.dataset.revive;
    var saved = {};
    try { saved = JSON.parse(get("local", key) || "{}") || {}; } catch (e) { saved = {}; }
    box.querySelectorAll("input[data-step]").forEach(function (cb) {
      cb.checked = !!saved[cb.dataset.step];
      cb.addEventListener("change", function () {
        saved[cb.dataset.step] = cb.checked;
        set("local", key, JSON.stringify(saved));
      });
    });
  }

  function fmt(sec) {
    var m = Math.floor(sec / 60), s = Math.max(0, Math.floor(sec % 60));
    return m + ":" + (s < 10 ? "0" : "") + s;
  }
  window.QD.fmt = fmt;

  /* ---- 2-minute timer ---- */
  function timer(box) {
    if (!once(box, "timerReady")) return;
    var total = parseInt(box.dataset.timer || "120", 10);
    var btn = box.querySelector("[data-timer-start]");
    var bar = box.querySelector("[data-timer-bar]");
    var out = box.querySelector("[data-timer-out]");
    var t = null;
    btn.addEventListener("click", function () {
      if (t) return;
      var end = Date.now() + total * 1000;
      var lastSpoken = -1;
      btn.disabled = true;
      bar.hidden = false;
      function tick() {
        var left = Math.max(0, Math.round((end - Date.now()) / 1000));
        bar.value = total - left;
        // Announce at most every 30 s so screen readers aren't flooded.
        var bucket = Math.ceil(left / 30);
        if (bucket !== lastSpoken || left === 0) {
          lastSpoken = bucket;
          out.textContent = left > 0 ? box.dataset.timerLeft.replace("{x}", fmt(left)) : box.dataset.timerDone;
        }
        if (left <= 0) { clearInterval(t); t = null; btn.disabled = false; }
      }
      tick();
      t = setInterval(tick, 1000);
    });
  }

  /* ---- buttons/text that wait N seconds (resend link, daily cap) ---- */
  function countdown(el) {
    if (!once(el, "countdownReady")) return;
    var left = parseInt(el.dataset.countdown || "0", 10);
    if (!(left > 0)) return;
    var isBtn = el.tagName === "BUTTON";
    var form = el.closest("form");
    var out = form ? form.querySelector("[data-countdown-out]") : null;
    var tpl = el.dataset.countdownText || "";
    function tick() {
      if (left <= 0) {
        if (isBtn) el.disabled = false;
        if (out) out.textContent = "";
        clearInterval(t);
        return;
      }
      if (isBtn) el.disabled = true;
      if (out && tpl) out.textContent = tpl.replace("{x}", left);
      left -= 1;
    }
    var t = setInterval(tick, 1000);
    tick();
  }

  /* ---- drop form draft: survives a lost session (§6, 401) ---- */
  function draft(form) {
    if (!once(form, "draftReady")) return;
    var key = form.dataset.draft;
    var fields = ["title", "one_liner", "link_url", "skills", "time_bucket", "energy", "voice_url"];
    var saved = null;
    try { saved = JSON.parse(get("session", key) || "null"); } catch (e) { saved = null; }
    var hasServerValues = fields.some(function (n) {
      var el = form.elements[n];
      if (!el) return false;
      if (el instanceof RadioNodeList) return !!el.value;
      return !!el.value;
    });
    if (saved && !hasServerValues) {
      fields.forEach(function (n) {
        var el = form.elements[n];
        if (!el || saved[n] == null) return;
        if (el instanceof RadioNodeList) {
          Array.prototype.forEach.call(el, function (r) { r.checked = r.value === saved[n]; });
        } else if (n !== "voice_url") {
          el.value = saved[n];
        }
      });
    }
    function save() {
      var data = {};
      fields.forEach(function (n) { var el = form.elements[n]; if (el) data[n] = el.value; });
      set("session", key, JSON.stringify(data));
    }
    form.addEventListener("input", save);
    form.addEventListener("change", save);
  }

  function clearDraft(el) {
    if (!once(el, "clearReady")) return;
    del("session", el.dataset.clearDraft);
  }

  /* ---- report: the third report hides the quest -> fade its card ---- */
  function hideCard(el) {
    if (!once(el, "hideReady")) return;
    var card = document.getElementById(el.dataset.hideCard);
    if (!card) return;
    setTimeout(function () {
      card.classList.add("is-gone");
      setTimeout(function () { card.remove(); }, 400);
    }, 1600);
  }

  function autofocus(el) {
    if (!once(el, "afReady")) return;
    el.focus({ preventScroll: false });
  }

  /* Without HTMX, still stop double submits. */
  function lock(form) {
    if (!once(form, "lockReady")) return;
    form.addEventListener("submit", function (e) {
      if (window.htmx) return;
      if (form.dataset.sent) { e.preventDefault(); return; }
      form.dataset.sent = "1";
      form.querySelectorAll("button[type=submit], button:not([type])").forEach(function (b) { b.disabled = true; });
    });
  }

  function init(root) {
    each(root, "input[data-chips]", chips);
    each(root, "[data-revive]", revive);
    each(root, "[data-timer]", timer);
    each(root, "[data-countdown]", countdown);
    each(root, "form[data-draft]", draft);
    each(root, "[data-clear-draft]", clearDraft);
    each(root, "[data-hide-card]", hideCard);
    each(root, "[data-autofocus]", autofocus);
    each(root, "form[method=post]", lock);
    if (window.QD.recorder) each(root, "[data-recorder]", window.QD.recorder);
  }
  window.QD.init = init;

  function start() {
    if (window.htmx) {
      window.htmx.onLoad(init);
      // After a boosted navigation, move focus to the new page (a11y).
      document.body.addEventListener("htmx:afterSettle", function (e) {
        if (e.detail && e.detail.boosted) {
          var main = document.getElementById("main");
          var af = main && main.querySelector("[data-autofocus]");
          if (main && !af) main.focus({ preventScroll: true });
        }
      });
    } else {
      init(document.body);
    }
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();
})();

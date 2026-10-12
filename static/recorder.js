/* Voice note recorder (docs/web-ui.md §6).
   Records Opus (webm/ogg) or AAC (mp4) with MediaRecorder, enforces the
   10-60 s rule (the server cannot check duration), uploads to /v1/voice
   with progress and writes the returned voice_url into the hidden field.
   The plain file input stays available as the fallback, and without JS
   the file simply posts with the form. */
(function () {
  "use strict";
  var MIN = 10, MAX = 60;
  var TYPES = ["audio/webm;codecs=opus", "audio/ogg;codecs=opus", "audio/mp4", "audio/webm"];

  function pickType() {
    if (!window.MediaRecorder || !MediaRecorder.isTypeSupported) return "";
    for (var i = 0; i < TYPES.length; i++) if (MediaRecorder.isTypeSupported(TYPES[i])) return TYPES[i];
    return "";
  }
  function extFor(type) {
    if (type.indexOf("ogg") >= 0) return "ogg";
    if (type.indexOf("mp4") >= 0) return "m4a";
    return "webm";
  }
  function fmt(s) {
    s = Math.max(0, Math.floor(s));
    return Math.floor(s / 60) + ":" + (s % 60 < 10 ? "0" : "") + (s % 60);
  }

  function recorder(box) {
    if (box.dataset.recReady) return;
    box.dataset.recReady = "1";
    var $ = function (sel) { return box.querySelector(sel); };
    var msg = function (k) { return box.getAttribute("data-msg-" + k) || ""; };
    var form = box.closest("form");
    var urlField = $("[data-rec-url]");
    var live = $("[data-rec-live]");
    var done = $("[data-rec-done]");
    var finalAudio = $("[data-rec-final]");
    var toggle = $("[data-rec-toggle]");
    var label = $("[data-rec-label]");
    var icon = $("[data-rec-icon]");
    var timeEl = $("[data-rec-time]");
    var level = $("[data-rec-level]");
    var review = $("[data-rec-review]");
    var preview = $("[data-rec-preview]");
    var fileWrap = $("[data-rec-file]");
    var fileInput = fileWrap.querySelector("input[type=file]");
    var orLabel = $("[data-rec-or]");
    var progress = $("[data-rec-progress]");
    var status = $("[data-rec-status]");
    var canRecord = !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia && pickType());

    var rec = null, stream = null, chunks = [], started = 0, tickT = null, raf = 0, ctx = null;
    var blob = null, uploading = false;

    function say(text, isErr) {
      status.textContent = text || "";
      status.classList.toggle("is-err", !!isErr);
    }
    function showInputs() {
      done.hidden = true;
      fileWrap.hidden = false;
      if (canRecord) { live.hidden = false; orLabel.hidden = false; }
    }
    function showDone(url) {
      urlField.value = url;
      urlField.dispatchEvent(new Event("change", { bubbles: true }));
      finalAudio.src = url;
      done.hidden = false;
      live.hidden = true;
      fileWrap.hidden = true;
      review.hidden = true;
      fileInput.value = "";
      say(msg("ready"));
    }

    if (urlField.value) { done.hidden = false; live.hidden = true; fileWrap.hidden = true; }
    else showInputs();

    $("[data-rec-replace]").addEventListener("click", function () {
      urlField.value = "";
      finalAudio.removeAttribute("src");
      say("");
      showInputs();
    });

    function stopTracks() {
      if (stream) stream.getTracks().forEach(function (t) { t.stop(); });
      stream = null;
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
      if (ctx) { try { ctx.close(); } catch (e) { /* ignore */ } ctx = null; }
      level.style.width = "0";
    }
    function setRecording(on) {
      toggle.setAttribute("aria-pressed", on ? "true" : "false");
      label.textContent = on ? msg("stop") : msg("record");
      icon.setAttribute("href", on ? "#i-stop" : "#i-mic");
    }
    function meter() {
      try {
        var AC = window.AudioContext || window.webkitAudioContext;
        if (!AC) return;
        ctx = new AC();
        var src = ctx.createMediaStreamSource(stream);
        var an = ctx.createAnalyser();
        an.fftSize = 512;
        src.connect(an);
        var buf = new Uint8Array(an.fftSize);
        (function loop() {
          an.getByteTimeDomainData(buf);
          var peak = 0;
          for (var i = 0; i < buf.length; i++) peak = Math.max(peak, Math.abs(buf[i] - 128));
          level.style.width = Math.min(100, Math.round(peak / 128 * 180)) + "%";
          raf = requestAnimationFrame(loop);
        })();
      } catch (e) { /* meter is decoration */ }
    }

    function start() {
      say("");
      review.hidden = true;
      navigator.mediaDevices.getUserMedia({ audio: true }).then(function (s) {
        stream = s;
        var type = pickType();
        chunks = [];
        try { rec = new MediaRecorder(s, { mimeType: type }); } catch (e) { rec = new MediaRecorder(s); }
        rec.ondataavailable = function (e) { if (e.data && e.data.size) chunks.push(e.data); };
        rec.onstop = function () {
          var secs = (Date.now() - started) / 1000;
          clearInterval(tickT);
          stopTracks();
          setRecording(false);
          var t = (rec && rec.mimeType) || type || "audio/webm";
          if (secs < MIN) { blob = null; say(msg("too-short"), true); timeEl.textContent = "0:00"; return; }
          blob = new Blob(chunks, { type: t });
          preview.src = URL.createObjectURL(blob);
          review.hidden = false;
          say("");
        };
        started = Date.now();
        rec.start(250);
        setRecording(true);
        meter();
        tickT = setInterval(function () {
          var secs = (Date.now() - started) / 1000;
          timeEl.textContent = fmt(secs);
          if (secs >= MAX && rec && rec.state === "recording") rec.stop();
        }, 200);
      }).catch(function () {
        say(msg("no-mic"), true);
        live.hidden = true;
        orLabel.hidden = true;
        fileInput.focus();
      });
    }

    toggle.addEventListener("click", function () {
      if (rec && rec.state === "recording") rec.stop();
      else start();
    });
    $("[data-rec-again]").addEventListener("click", function () {
      review.hidden = true;
      blob = null;
      timeEl.textContent = "0:00";
      start();
    });
    $("[data-rec-use]").addEventListener("click", function () {
      if (!blob) return;
      upload(blob, "note." + extFor(blob.type));
    });

    function durationOk(file) {
      return new Promise(function (resolve) {
        try {
          var a = document.createElement("audio");
          var u = URL.createObjectURL(file);
          var t = setTimeout(function () { resolve(true); }, 4000);
          a.preload = "metadata";
          a.onloadedmetadata = function () {
            clearTimeout(t);
            URL.revokeObjectURL(u);
            var d = a.duration;
            // Some containers report Infinity/NaN until fully read: allow.
            resolve(!isFinite(d) || (d >= MIN - 0.5 && d <= MAX + 1));
          };
          a.onerror = function () { clearTimeout(t); resolve(true); };
          a.src = u;
        } catch (e) { resolve(true); }
      });
    }

    fileInput.addEventListener("change", function () {
      var f = fileInput.files && fileInput.files[0];
      if (!f) return;
      if (f.size > 5 * 1024 * 1024) { say(msg("too-big"), true); fileInput.value = ""; return; }
      durationOk(f).then(function (ok) {
        if (!ok) { say(msg("duration"), true); fileInput.value = ""; return; }
        upload(f, f.name);
      });
    });

    function upload(file, name) {
      uploading = true;
      progress.hidden = false;
      progress.value = 0;
      say(msg("uploading"));
      var fd = new FormData();
      fd.append("file", file, name);
      var x = new XMLHttpRequest();
      x.open("POST", "/v1/voice");
      x.upload.onprogress = function (e) { if (e.lengthComputable) progress.value = Math.round(e.loaded / e.total * 100); };
      x.onload = function () {
        uploading = false;
        progress.hidden = true;
        var body = {};
        try { body = JSON.parse(x.responseText || "{}"); } catch (e) { body = {}; }
        if (x.status === 201 && body.voice_url) { showDone(body.voice_url); return; }
        if (x.status === 401) {
          say(msg("need-login"), true);
          // The draft is already in sessionStorage (app.js); come back here.
          setTimeout(function () { window.location.href = "/login?next=" + encodeURIComponent("/drop"); }, 1800);
          return;
        }
        var m = x.status === 413 ? "too-big" : x.status === 507 ? "full" : x.status === 400 ? "bad-type" : "failed";
        say(msg(m), true);
        fileInput.value = "";
      };
      x.onerror = function () { uploading = false; progress.hidden = true; say(msg("failed"), true); };
      x.send(fd);
    }

    if (form) {
      form.addEventListener("submit", function (e) {
        if (uploading) {
          e.preventDefault();
          e.stopImmediatePropagation();
          say(msg("wait"), true);
        }
      }, true);
      // Uploaded already? Don't send the same bytes again with the form.
      form.addEventListener("htmx:configRequest", function (e) {
        if (urlField.value && e.detail && e.detail.formData && e.detail.formData.delete) e.detail.formData.delete("voice_file");
      });
    }
  }

  window.QD = window.QD || {};
  window.QD.recorder = recorder;
  function boot() { document.querySelectorAll("[data-recorder]").forEach(recorder); }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", boot);
  else boot();
})();

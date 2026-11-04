"use strict";
// Temporary browser test client. The native Android app replaces it and speaks the same API.
const $ = (id) => document.getElementById(id);
const KEY = "bridge.token";
const CHUNK = 8 * 1024 * 1024; // each PUT carries up to 8 MiB; a failed chunk is retried from where the PC says it is
let token = null;
let lastId = null;
let delSeen = null; // how many deletions this page has already applied
let polling = 0; // generation counter; bumping it stops an old poll loop

function getToken() {
  try { return localStorage.getItem(KEY); } catch (e) { return null; }
}
function setToken(v) {
  try { v ? localStorage.setItem(KEY, v) : localStorage.removeItem(KEY); } catch (e) { /* private mode */ }
}

async function api(path, body, method) {
  const headers = { Authorization: "Bearer " + token };
  const opt = { headers, method: method || "GET" };
  if (body !== undefined) {
    opt.method = method || "POST";
    headers["Content-Type"] = "application/json";
    opt.body = JSON.stringify(body);
  }
  const res = await fetch(path, opt);
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    const err = new Error(data.error || "HTTP " + res.status);
    err.status = res.status;
    err.data = data;
    throw err;
  }
  return data;
}

function show(paired) {
  $("pairCard").classList.toggle("hidden", paired);
  $("sendCard").classList.toggle("hidden", !paired);
  $("notesCard").classList.toggle("hidden", !paired);
  $("dot").classList.toggle("ok", paired);
  $("status").textContent = paired ? "Connected" : "Not paired";
}

function fmtTime(ts) {
  return new Date(ts * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function fmtSize(n) {
  if (n < 1024) return n + " B";
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return (v >= 100 ? v.toFixed(0) : v.toFixed(1)) + " " + units[i];
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (e) {
    const ta = document.createElement("textarea"); // fallback where the async API is unavailable
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "absolute";
    ta.style.left = "-9999px";
    document.body.append(ta);
    ta.select();
    let ok = false;
    try { ok = document.execCommand("copy"); } catch (e2) { /* ignore */ }
    ta.remove();
    return ok;
  }
}

// ---------------------------------------------------------------- notes feed

function button(label, onClick, cls) {
  const b = document.createElement("button");
  b.className = "push" + (cls ? " " + cls : "");
  b.textContent = label;
  b.addEventListener("click", () => onClick(b));
  return b;
}

// Download inside the page (same connection and certificate exception as everything else here).
// The data is collected in memory, so this is meant for test-sized files; the native app streams to disk.
const MAX_IN_PAGE = 600 * 1024 * 1024;

async function downloadInPage(f, btn) {
  if (f.size > MAX_IN_PAGE) { btn.textContent = "Too big for this page, use 'Direct link'"; return; }
  btn.disabled = true;
  try {
    const res = await fetch("/api/files/" + f.id + "/content", { headers: { Authorization: "Bearer " + token } });
    if (!res.ok) { const e = new Error("HTTP " + res.status); e.status = res.status; throw e; }
    const reader = res.body.getReader();
    const parts = [];
    let got = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      parts.push(value);
      got += value.length;
      btn.textContent = Math.round((got / Math.max(f.size, 1)) * 100) + "%";
    }
    if (got !== f.size) throw new Error("incomplete: " + got + " of " + f.size + " bytes");
    const url = URL.createObjectURL(new Blob(parts, { type: "application/octet-stream" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = f.name;
    document.body.append(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 120000);
    btn.textContent = "Saved, again?";
  } catch (e) {
    btn.textContent = e.status === 410 ? "File no longer on PC" : "Failed: " + e.message;
  }
  btn.disabled = false;
}

// Plain link: the browser's own downloader fetches it. Kept for comparison.
async function downloadFile(id, btn) {
  btn.textContent = "Starting…";
  try {
    const t = await api("/api/files/" + id + "/ticket", {});
    const a = document.createElement("a");
    a.href = t.url; // short-lived link: the browser downloads and can resume it by itself
    a.rel = "noopener";
    document.body.append(a);
    a.click();
    a.remove();
    btn.textContent = "Direct link again";
  } catch (e) {
    btn.textContent = e.status === 410 ? "File no longer on PC" : "Link failed";
  }
}

function noteEl(n) {
  const box = document.createElement("div");
  box.className = "note";
  box.dataset.id = n.id;
  box.dataset.mid = n.mid || "";
  const meta = document.createElement("div");
  meta.className = "meta";
  const who = document.createElement("span");
  const toPhone = n.src === "pc";
  who.textContent = (toPhone ? "PC to phone" : n.src + " to PC") + " · " + fmtTime(n.ts);
  meta.append(who);
  const text = document.createElement("div");
  text.className = "text";
  if (n.kind === "file" && n.file) {
    text.textContent = n.file.name + " · " + fmtSize(n.file.size);
    const sum = document.createElement("div");
    sum.className = "muted mono";
    sum.textContent = "SHA-256 " + n.file.sha256.slice(0, 16) + "…";
    box.append(meta, text, sum);
    if (n.file.dir === "to_phone") {
      meta.append(button("Download", (b) => downloadInPage(n.file, b)));
      meta.append(button("Direct link", (b) => downloadFile(n.file.id, b)));
    }
    else { const s = document.createElement("span"); s.className = "push"; s.textContent = "Saved on PC"; meta.append(s); }
    return box;
  }
  text.textContent = n.text || ""; // never interpret note text as HTML
  meta.append(button("Copy", async (b) => {
    b.textContent = (await copyText(n.text)) ? "Copied" : "Copy failed";
    setTimeout(() => (b.textContent = "Copy"), 1200);
  }));
  box.append(meta);
  if (n.reply) {
    const q = document.createElement("div");
    q.className = "quote";
    q.textContent = (n.reply.src === "pc" ? "PC" : n.reply.src) + ": " + n.reply.preview;
    box.append(q);
  }
  box.append(text);
  return box;
}

function addNotes(notes) {
  const list = $("notes");
  if (notes.length && list.querySelector(".muted")) list.textContent = "";
  for (const n of notes) {
    if (list.querySelector(`[data-id="${n.id}"]`)) continue;
    list.prepend(noteEl(n));
    lastId = Math.max(lastId ?? 0, n.id);
  }
}

async function poll() {
  const mine = ++polling;
  let backoff = 1000;
  while (mine === polling) {
    try {
      const q = lastId === null ? "?wait=0" : "?since=" + lastId + "&wait=10&del=" + (delSeen ?? 0);
      const data = await api("/api/notes" + q);
      if (mine !== polling) return;
      addNotes(data.notes);
      // messages deleted on the PC disappear here too
      for (const mid of data.deleted || []) {
        for (const el of $("notes").querySelectorAll("[data-mid]")) if (el.dataset.mid === mid) el.remove();
      }
      if (typeof data.del === "number") delSeen = data.del;
      if (lastId === null) lastId = data.last_id;
      $("status").textContent = "Connected";
      $("dot").classList.add("ok");
      backoff = 1000;
    } catch (e) {
      if (e.status === 401) { unpair("This phone was unpaired on the PC. Scan a new code."); return; }
      $("status").textContent = "PC not reachable";
      $("dot").classList.remove("ok");
      await sleep(backoff);
      backoff = Math.min(backoff * 2, 15000);
    }
  }
}

function unpair(message) {
  polling++;
  token = null;
  lastId = null;
  delSeen = null;
  setToken(null);
  $("notes").textContent = "";
  $("uploads").textContent = "";
  show(false);
  $("pairErr").textContent = message || "";
}

// ---------------------------------------------------------------- pairing

async function pair(code, name) {
  $("pairErr").textContent = "";
  try {
    const res = await fetch("/api/pair", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ code, name }),
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) {
      $("pairErr").textContent =
        data.error === "locked" ? "Too many wrong codes. Wait a minute and show a new code on the PC."
        : "That code is wrong or expired. Show a new code on the PC.";
      return;
    }
    token = data.token;
    setToken(token);
    history.replaceState(null, "", location.pathname); // drop the code from the address bar
    show(true);
    poll();
  } catch (e) {
    $("pairErr").textContent = "Could not reach the PC.";
  }
}

// ---------------------------------------------------------------- sending files

function putChunk(id, offset, blob, onProgress) {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("PUT", "/api/files/" + id + "?offset=" + offset);
    xhr.setRequestHeader("Authorization", "Bearer " + token);
    xhr.upload.onprogress = (e) => onProgress(e.loaded);
    xhr.onload = () => {
      let data = {};
      try { data = JSON.parse(xhr.responseText); } catch (e) { /* not json */ }
      if (xhr.status === 200) resolve(data);
      else { const err = new Error(data.error || "HTTP " + xhr.status); err.status = xhr.status; err.data = data; reject(err); }
    };
    xhr.onerror = () => { const err = new Error("network"); err.status = 0; reject(err); };
    xhr.ontimeout = xhr.onerror;
    xhr.send(blob);
  });
}

function uploadRow(file) {
  const row = document.createElement("div");
  row.className = "note";
  const name = document.createElement("div");
  name.className = "text";
  name.textContent = file.name + " · " + fmtSize(file.size);
  const bar = document.createElement("progress");
  bar.max = Math.max(file.size, 1);
  bar.value = 0;
  const label = document.createElement("div");
  label.className = "muted";
  label.textContent = "Waiting…";
  row.append(name, bar, label);
  $("uploads").prepend(row);
  return { row, bar, label };
}

async function uploadFile(file) {
  const ui = uploadRow(file);
  const started = Date.now();
  const report = (done) => {
    ui.bar.value = done;
    const secs = (Date.now() - started) / 1000;
    const speed = secs > 0.5 ? " · " + fmtSize(done / secs) + "/s" : "";
    ui.label.textContent = Math.round((done / Math.max(file.size, 1)) * 100) + "%" + speed;
  };
  try {
    const created = await api("/api/files", { name: file.name, size: file.size });
    // The PC's user decides first; nothing is sent until the PC says yes.
    let state = created.state;
    ui.label.textContent = state === "pending" ? "Waiting for the PC to accept…" : ui.label.textContent;
    while (state === "pending") {
      await sleep(1000);
      state = (await api("/api/files/" + created.id)).state;
    }
    if (state === "declined") throw new Error("declined");
    if (state === "cancelled") throw new Error("cancelled");
    if (!created.done) {
      let offset = 0, fails = 0;
      while (offset < file.size) {
        const end = Math.min(offset + CHUNK, file.size);
        try {
          await putChunk(created.id, offset, file.slice(offset, end), (loaded) => report(offset + loaded));
          offset = end;
          fails = 0;
          report(offset);
          if (offset >= file.size) ui.label.textContent = "Checking on PC…";
        } catch (e) {
          if (e.status === 401 || e.status === 404 || e.status === 413 || e.status === 422 || e.status === 507) throw e;
          if (++fails > 6) throw e;
          ui.label.textContent = "Connection lost, retrying (" + fails + ")…";
          await sleep(1000 * fails);
          try { offset = (await api("/api/files/" + created.id)).received; } catch (e2) { /* retry the same chunk */ }
        }
      }
    }
    ui.bar.value = file.size;
    ui.label.textContent = "Saved on PC";
  } catch (e) {
    if (e.status === 401) { unpair("This phone was unpaired on the PC. Scan a new code."); return; }
    const reasons = { declined: "The PC declined the file.", cancelled: "Cancelled on the PC.", hash_mismatch: "The file arrived damaged.", disk_full: "The PC disk is full.", file_too_large: "File is too large.", too_many_transfers: "Too many transfers at once." };
    ui.label.textContent = "Failed: " + (reasons[e.message] || e.message);
    ui.label.className = "err";
  }
}

let uploadChain = Promise.resolve(); // one file at a time keeps progress readable

// ---------------------------------------------------------------- wiring

$("pairBtn").addEventListener("click", () => {
  const code = $("code").value.trim();
  if (!code) { $("pairErr").textContent = "Enter the code from the PC screen."; return; }
  pair(code, $("devName").value.trim() || "Phone");
});

$("sendBtn").addEventListener("click", async () => {
  $("sendErr").textContent = "";
  const text = $("text").value;
  if (!text.trim()) { $("sendErr").textContent = "Enter some text first."; return; }
  try {
    await api("/api/notes", { text });
    $("text").value = "";
  } catch (e) {
    if (e.status === 401) unpair("This phone was unpaired on the PC. Scan a new code.");
    else $("sendErr").textContent = e.message === "text_too_long" ? "That text is too long." : "Could not send: " + e.message;
  }
});

$("pasteBtn").addEventListener("click", async () => {
  try {
    $("text").value = await navigator.clipboard.readText();
  } catch (e) {
    $("sendErr").textContent = "Paste is blocked here. Long-press the box and choose Paste.";
  }
});

$("filesBtn").addEventListener("click", () => $("fileInput").click());
$("fileInput").addEventListener("change", () => {
  for (const f of $("fileInput").files) uploadChain = uploadChain.then(() => uploadFile(f));
  $("fileInput").value = "";
});

$("unpairBtn").addEventListener("click", () => {
  if (confirm("Unpair this phone from the PC?")) unpair("");
});

// Browsers do not know the name the user gave the phone, but Chrome can tell the model
// (for example "Pixel 8") when asked. The PC window lets the user rename the phone afterwards.
async function guessPhoneName() {
  const fallback = navigator.userAgentData && navigator.userAgentData.mobile ? "Android phone" : "Phone";
  try {
    const h = await navigator.userAgentData.getHighEntropyValues(["model"]);
    const model = (h.model || "").trim();
    if (model.length > 1) return model.slice(0, 40);
  } catch (e) { /* not supported: use the fallback */ }
  return fallback;
}

(async function init() {
  const params = new URLSearchParams(location.hash.slice(1));
  $("devName").value = await guessPhoneName();
  token = getToken();
  if (params.get("pair")) {
    token = null; // a fresh code always wins over an old token
    setToken(null);
    show(false);
    pair(params.get("pair"), $("devName").value);
  } else if (token) {
    show(true);
    poll();
  } else {
    show(false);
  }
})();

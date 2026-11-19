"use strict";

// toasts: from `HX-Trigger: {"toast": {"level": "success", "message": "..."}}` or showToast()
function showToast(level, message) {
  const box = document.getElementById("toasts");
  if (!box) return;
  const t = document.createElement("div");
  t.className = "toast " + level;
  t.textContent = message;
  box.appendChild(t);
  setTimeout(() => t.remove(), 4000);
}

document.addEventListener("toast", (e) => showToast(e.detail.level || "info", e.detail.message || ""));
document.addEventListener("htmx:responseError", (e) => {
  const text = (e.detail.xhr && e.detail.xhr.responseText) || "Request failed";
  showToast("error", text.length > 200 ? "Request failed" : text);
});

// sidebar: hidden or shown, remembered on desktop
function toggleMenu() {
  if (window.innerWidth > 991) {
    const hidden = document.body.classList.toggle("menu-hidden");
    try { localStorage.setItem("STATIC_MENU_DESKTOP_INACTIVE", hidden ? "1" : "0"); } catch (_) {}
  } else {
    document.body.classList.toggle("menu-open");
  }
}

document.addEventListener("DOMContentLoaded", () => {
  try {
    if (localStorage.getItem("STATIC_MENU_DESKTOP_INACTIVE") === "1") document.body.classList.add("menu-hidden");
  } catch (_) {}
  if (document.getElementById("swarm")) swarmInit();
});

// logs: last line number, scrolling
let logsScroll = true;
function lastLogId() {
  const lines = document.querySelectorAll("#logs [data-id]");
  return lines.length ? lines[lines.length - 1].dataset.id : 0;
}
function toggleLogScroll(button) {
  logsScroll = !logsScroll;
  button.textContent = logsScroll ? "Stop Scrolling" : "Start Scrolling";
}
function clearLogs() {
  const logs = document.getElementById("logs");
  const last = lastLogId();
  logs.innerHTML = "";
  const marker = document.createElement("span");
  marker.dataset.id = last;
  logs.appendChild(marker);
}
document.addEventListener("htmx:afterSwap", (e) => {
  if (e.detail.target.id === "logs") {
    const logs = e.detail.target;
    const filter = document.getElementById("log-filter");
    if (filter && filter.value) {
      logs.querySelectorAll("div[data-id]:not([data-checked])").forEach((line) => {
        line.dataset.checked = "1";
        if (!line.textContent.includes(filter.value)) line.remove();
      });
    }
    while (logs.children.length > 256) logs.firstElementChild.remove();
    if (logsScroll) logs.scrollTop = logs.scrollHeight;
  }
});

// firmware upload: the raw file in the request body
function uploadFirmware(input) {
  const file = input.files[0];
  if (!file) return;
  if (!file.name.endsWith(".bin")) {
    showToast("error", "Incorrect file, looking for a .bin firmware.");
    input.value = "";
    return;
  }
  const modal = document.getElementById("update-modal");
  const text = document.getElementById("update-text");
  const bar = document.getElementById("update-bar");
  modal.classList.remove("hidden");
  const xhr = new XMLHttpRequest();
  xhr.open("POST", "/api/system/OTA");
  xhr.setRequestHeader("Content-Type", "application/octet-stream");
  xhr.upload.onprogress = (e) => {
    if (e.lengthComputable) {
      const pct = Math.round((e.loaded / e.total) * 100);
      text.textContent = pct < 100 ? "Uploading... " + pct + "%" : "Applying update...";
      bar.style.width = pct + "%";
    }
  };
  xhr.onload = () => {
    if (xhr.status === 200) {
      text.textContent = "Firmware updated. The page will reload when the device comes back online.";
      waitForDevice();
    } else {
      modal.classList.add("hidden");
      showToast("error", xhr.responseText || "Update failed");
    }
  };
  xhr.onerror = () => {
    modal.classList.add("hidden");
    showToast("error", "Update failed");
  };
  xhr.send(file);
  input.value = "";
}

function waitForDevice() {
  setTimeout(() => {
    fetch("/api/system/info", { cache: "no-store" })
      .then((r) => (r.ok ? location.reload() : waitForDevice()))
      .catch(waitForDevice);
  }, 3000);
}

// theme: a swatch sets the colour field and submits
function pickColor(color) {
  const input = document.getElementById("primary-color");
  input.value = color;
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

// swarm: devices kept in this browser, read from their own API
const SWARM_KEY = "SWARM_DATA";
let swarm = [];
let swarmTimer = null;

function swarmLoad() {
  try { swarm = JSON.parse(localStorage.getItem(SWARM_KEY) || "[]"); } catch (_) { swarm = []; }
  if (!swarm.includes(location.host)) swarm.unshift(location.host);
}
function swarmSave() {
  try { localStorage.setItem(SWARM_KEY, JSON.stringify(swarm)); } catch (_) {}
}
function swarmInit() {
  swarmLoad();
  swarmRefresh();
  swarmTimer = setInterval(swarmRefresh, 30000);
}
function swarmAdd() {
  const input = document.getElementById("swarm-address");
  const address = input.value.trim().replace(/^https?:\/\//, "").replace(/\/.*$/, "");
  if (!address) return;
  if (!swarm.includes(address)) swarm.push(address);
  swarmSave();
  input.value = "";
  swarmRefresh();
}
function swarmRemove(address) {
  swarm = swarm.filter((a) => a !== address);
  swarmSave();
  swarmRefresh();
}
async function swarmFetch(address, timeout) {
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), timeout);
  try {
    const r = await fetch("http://" + address + "/api/system/info", { signal: ctl.signal, cache: "no-store" });
    return r.ok ? await r.json() : null;
  } catch (_) {
    return null;
  } finally {
    clearTimeout(timer);
  }
}
async function swarmScan(button) {
  const ip = (document.getElementById("swarm").dataset.ip || "").split(".");
  if (ip.length !== 4) return;
  button.disabled = true;
  const prefix = ip.slice(0, 3).join(".") + ".";
  const found = [];
  const addresses = [];
  for (let i = 1; i < 255; i++) addresses.push(prefix + i);
  for (let i = 0; i < addresses.length; i += 32) {
    const batch = addresses.slice(i, i + 32);
    const results = await Promise.all(batch.map((a) => swarmFetch(a, 3000).then((info) => (info && info.ASICModel ? a : null))));
    results.forEach((a) => a && found.push(a));
  }
  found.forEach((a) => { if (!swarm.includes(a)) swarm.push(a); });
  swarmSave();
  button.disabled = false;
  showToast("success", found.length + " device(s) found");
  swarmRefresh();
}
function hashSuffix(ghs) {
  const units = [" H/s", " Kh/s", " Mh/s", " Gh/s", " Th/s", " Ph/s", " Eh/s"];
  if (!(ghs > 0)) return "0 H/s";
  let v = ghs * 1e9, u = 0;
  while (v >= 1000 && u < units.length - 1) { v /= 1000; u++; }
  return v.toFixed(v < 10 ? 2 : v < 100 ? 1 : 0) + units[u];
}
function diffSuffix(d) {
  const s = ["", " K", " M", " G", " T", " P", " E"];
  if (!(d >= 1000)) return String(Math.floor(d || 0));
  let u = 0;
  while (d >= 1000 && u < s.length - 1) { d /= 1000; u++; }
  return d.toFixed(2) + s[u];
}
function escapeHtml(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}
async function swarmPost(address, action) {
  try {
    const r = await fetch("http://" + address + "/api/system/" + action, { method: "POST" });
    const body = await r.json().catch(() => ({}));
    showToast(r.ok ? "success" : "error", body.message || action);
  } catch (_) {
    showToast("error", "Failed to reach " + address);
  }
  setTimeout(swarmRefresh, 1000);
}
async function swarmRefresh() {
  const results = await Promise.all(swarm.map((a) => swarmFetch(a, 5000).then((info) => ({ address: a, info }))));
  let hashrate = 0, power = 0, best = 0, online = 0;
  const filter = (document.getElementById("swarm-filter") || {}).value || "";
  const rows = results
    .filter(({ address, info }) => !filter || address.includes(filter) || (info && (info.hostname || "").includes(filter)))
    .map(({ address, info }) => {
      if (!info) {
        return "<tr><td>" + escapeHtml(address) + '</td><td colspan="8" class="muted">Offline</td><td><button class="btn btn-secondary" onclick="swarmRemove(\'' + escapeHtml(address) + "')\">Remove</button></td></tr>";
      }
      online++;
      hashrate += info.hashRate || 0;
      power += info.power || 0;
      best = Math.max(best, info.bestDiff || 0);
      const temps = [info.temp, info.temp2, info.vrTemp].filter((t) => t > 0).map((t) => t.toFixed(1)).join(" / ");
      const notes = [];
      if (info.miningPaused) notes.push("Paused");
      if (info.overheat_mode) notes.push("Overheated");
      if (info.isUsingFallbackStratum) notes.push("Fallback Pool");
      if (info.showNewBlock) notes.push("Block found");
      const here = address === location.host ? " ★" : "";
      return (
        "<tr><td><a href=\"http://" + escapeHtml(address) + '/" target="_blank">' + escapeHtml(info.hostname || address) + "</a>" + here +
        '<div class="muted">' + escapeHtml(info.ipv4 || address) + " • " + escapeHtml(info.version || "") + "</div>" +
        (notes.length ? '<div class="yellow">' + escapeHtml(notes.join(", ")) + "</div>" : "") + "</td>" +
        "<td>" + hashSuffix(info.hashRate) + "</td>" +
        "<td>" + (info.sharesAccepted || 0) + ' <span class="muted">/</span> <span class="red">' + (info.sharesRejected || 0) + "</span></td>" +
        "<td>" + diffSuffix(info.bestSessionDiff) + ' <span class="muted">/</span> ' + diffSuffix(info.bestDiff) + "</td>" +
        "<td>" + Math.floor((info.uptimeSeconds || 0) / 3600) + "h</td>" +
        "<td>" + (info.power || 0).toFixed(1) + " W</td>" +
        "<td>" + (temps || "--") + " °C</td>" +
        "<td>" + diffSuffix(info.poolDifficulty) + "</td>" +
        "<td>" + escapeHtml(info.ASICModel || "") + "</td>" +
        '<td class="buttons" style="margin:0">' +
        '<button class="btn btn-secondary" onclick="swarmPost(\'' + escapeHtml(address) + "', '" + (info.miningPaused ? "resume" : "pause") + "')\">" + (info.miningPaused ? "Resume" : "Pause") + "</button>" +
        '<button class="btn btn-secondary" onclick="if(confirm(\'Restart ' + escapeHtml(info.hostname || address) + "?'))swarmPost('" + escapeHtml(address) + "', 'restart')\">Restart</button>" +
        '<button class="btn btn-secondary" onclick="swarmPost(\'' + escapeHtml(address) + "', 'identify')\">Identify</button>" +
        '<button class="btn btn-secondary" onclick="swarmRemove(\'' + escapeHtml(address) + "')\">Remove</button></td></tr>"
      );
    });
  document.getElementById("swarm-total-devices").textContent = online;
  document.getElementById("swarm-total-hashrate").textContent = hashSuffix(hashrate);
  document.getElementById("swarm-total-power").textContent = power.toFixed(1) + " W";
  document.getElementById("swarm-best").textContent = diffSuffix(best);
  document.getElementById("swarm-rows").innerHTML = rows.join("") ||
    '<tr><td colspan="10" class="muted">Scan your network for devices or add a device manually by using its address (IP or .local hostname).</td></tr>';
}

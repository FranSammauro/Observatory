const TOKEN_KEY = "obs_token";

const $ = (sel) => document.querySelector(sel);

const state = {
  token: sessionStorage.getItem(TOKEN_KEY) || "",
  ws: null,
  reconnectMs: 3000,
  summaryTimer: null,
  timeline: {},
};

function api(path) {
  return fetch(path, {
    headers: { Authorization: `Bearer ${state.token}` },
  }).then(async (res) => {
    if (res.status === 401) {
      showLogin();
      throw new Error("unauthorized");
    }
    if (!res.ok) {
      let code = res.status;
      try {
        code = (await res.json()).error?.code || res.status;
      } catch (_) {}
      throw new Error(String(code));
    }
    return res.json();
  });
}

function showLogin() {
  state.token = "";
  sessionStorage.removeItem(TOKEN_KEY);
  disconnectWs();
  $("#login").classList.remove("hidden");
  $("#app").classList.add("hidden");
}

function enterApp() {
  $("#login").classList.add("hidden");
  $("#app").classList.remove("hidden");
  switchView("#overview");
  refreshAll();
  pollAgents();
  connectWs();
}

function logout() {
  showLogin();
  $("#login-error").classList.add("hidden");
}

$("#login-form").addEventListener("submit", (e) => {
  e.preventDefault();
  const token = $("#token").value.trim();
  state.token = token;
  sessionStorage.setItem(TOKEN_KEY, token);
  $("#login-error").classList.add("hidden");
  api("/api/v1/health/summary")
    .then(() => enterApp())
    .catch((err) => {
      sessionStorage.removeItem(TOKEN_KEY);
      state.token = "";
      $("#login-error").textContent = `No se pudo validar el token (${err.message})`;
      $("#login-error").classList.remove("hidden");
    });
});

$("#logout").addEventListener("click", logout);

document.querySelectorAll(".nav-item").forEach((item) => {
  item.addEventListener("click", (e) => {
    e.preventDefault();
    switchView(item.getAttribute("href"));
  });
});

function switchView(hash) {
  document.querySelectorAll(".view").forEach((v) => v.classList.add("hidden"));
  document.querySelectorAll(".nav-item").forEach((n) => n.classList.remove("active"));
  const view = document.querySelector(`[id="view-${hash.slice(1)}"]`);
  if (view) view.classList.remove("hidden");
  const nav = document.querySelector(`[data-view="${hash.slice(1)}"]`);
  if (nav) nav.classList.add("active");
  window.location.hash = hash;
  if (hash === "#overview") refreshAll();
}

function markRefreshed() {
  $("#last-refresh").textContent = `ultima actualizacion ${nowStr()}`;
}

function refreshAll() {
  loadSummary();
  loadAgents();
  loadTimeline();
}

function loadSummary() {
  api("/api/v1/health/summary")
    .then((s) => {
      setCard("agents", s, ["total", "online", "degraded", "offline"]);
      setCard("checks", s, ["total", "up", "down", "unknown"]);
      setCard("alerts", s, ["total", "pending", "firing"]);
      markRefreshed();
    })
    .catch((err) => {
      if (err.message !== "unauthorized") markRefreshed();
    });
}

function setCard(prefix, data, keys) {
  keys.forEach((k) => {
    const el = $(`#${prefix}-${k}`);
    if (el) el.textContent = data[prefix]?.[k] ?? 0;
  });
}

function loadAgents() {
  api("/api/v1/agents")
    .then((j) => {
      const body = $("#agents-body");
      if (!j.agents) return;
      body.innerHTML = j.agents
        .map((a) => {
          const stateBadge = badge(a.state, a.state);
          const la = a.last_seen_age_secs ?? 0;
          return `<tr>
            <td class="mono">${shortId(a.agent_id)}</td>
            <td>${stateBadge}</td>
            <td title="${a.last_seen}">${relTime(la * 1000)}</td>
            <td title="${a.first_seen}">${absTime(a.first_seen)}</td>
          </tr>`;
        })
        .join("");
    })
    .catch(() => {});
}

$("#reload-agents").addEventListener("click", loadAgents);

function loadTimeline() {
  api("/api/v1/events/history?limit=50")
    .then((j) => {
      state.timeline = {};
      const ul = $("#timeline");
      ul.innerHTML = "";
      (j.events || []).forEach((ev) => prependEvent(ev));
    })
    .catch(() => {});
}

function prependEvent(ev) {
  const key = `${ev.ts}-${ev.check_id || ev.rule_id || ev.agent_id || ""}`;
  if (state.timeline[key]) return;
  state.timeline[key] = true;
  const li = document.createElement("li");
  li.innerHTML = `
    <span class="when" title="${ev.ts}">${relTime(tsMs(ev.ts))}</span>
    <span class="kind tag-${ev.type}">${ev.type.replace("_", " ")}</span>
    <span class="detail">${describe(ev)}</span>`;
  $("#timeline").prepend(li);
  while ($("#timeline").children.length > 100) {
    $("#timeline").lastChild.remove();
  }
}

function describe(ev) {
  switch (ev.type) {
    case "alert_event":
      return `${ev.rule_name} ${ev.from_state || "inactive"} \u2192 ${ev.to_state} ` +
        `(agent ${shortId(ev.agent_id)})`;
    case "health_result":
      return `${ev.check_name}: ${ev.ok ? "ok" : "down"} ` +
        `(${ev.detail}) ${ev.state_changed ? `· estado ${ev.state}` : ""}`;
    case "reboot_event":
      return `reboot detectado en ${shortId(ev.agent_id)} ` +
        `(uptime ${fmtUptime(ev.uptime_before)} \u2192 ${fmtUptime(ev.uptime_after)})`;
    case "connectivity_event":
      return `conectividad ${ev.from_state || "desconocido"} \u2192 ${ev.to_state} ` +
        `(agent ${shortId(ev.agent_id)})`;
    case "events_lagged":
      return `cliente atrasado: se descartaron ${ev.dropped} eventos`;
    default:
      return JSON.stringify(ev);
  }
}

function connectWs() {
  if (!state.token) return;
  disconnectWs();
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const ws = new WebSocket(`${proto}://${location.host}/api/v1/events?token=${encodeURIComponent(state.token)}`);
  state.ws = ws;
  ws.onmessage = (msg) => {
    const ev = JSON.parse(msg.data);
    if (!document.querySelector("#view-overview").classList.contains("hidden")) {
      prependEvent(ev);
      resetSummaryTimer();
      if (ev.type === "connectivity_event" && shortId(ev.agent_id)) loadAgentsDebounced();
    }
  };
  ws.onclose = () => {
    state.ws = null;
    if (!state.token) return;
    setTimeout(connectWs, state.reconnectMs);
  };
  ws.onerror = () => ws.close();
}

function disconnectWs() {
  if (state.ws) {
    state.ws.onclose = null;
    state.ws.close();
    state.ws = null;
  }
}

function resetSummaryTimer() {
  if (state.summaryTimer) clearTimeout(state.summaryTimer);
  state.summaryTimer = setTimeout(loadSummary, 800);
}

let agentsTimer = null;
function loadAgentsDebounced() {
  if (agentsTimer) return;
  agentsTimer = setTimeout(() => {
    agentsTimer = null;
    loadAgents();
  }, 1200);
}

let pollTimer = null;
function pollAgents() {
  if (pollTimer) clearInterval(pollTimer);
  pollTimer = setInterval(() => {
    if (state.token && !document.querySelector("#view-overview").classList.contains("hidden")) {
      loadAgents();
    }
  }, 10000);
}

function badge(text, cls) {
  return `<span class="badge ${cls}">${text}</span>`;
}

function shortId(uuid) {
  return (uuid || "").slice(0, 8);
}

function tsMs(ts) {
  if (!ts) return Date.now();
  const d = new Date(ts);
  return d.getTime() || Date.now();
}

function relTime(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 5) return "ahora";
  if (s < 60) return `hace ${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `hace ${m}m`;
  const h = Math.floor(m / 60);
  if (h < 48) return `hace ${h}h`;
  const d = Math.floor(h / 24);
  return `hace ${d}d`;
}

function absTime(ts) {
  return new Date(ts).toLocaleString();
}

function nowStr() {
  return new Date().toLocaleTimeString();
}

function fmtUptime(sec) {
  if (sec == null || sec === undefined) return "?";
  const days = Math.floor(sec / 86400);
  const h = Math.floor((sec % 86400) / 3600);
  if (days > 0) return `${days}d ${h}h`;
  const m = Math.floor((sec % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  return `${Math.floor(sec)}s`;
}

window.addEventListener("hashchange", () => {
  const hash = location.hash || "#overview";
  switchView(hash);
});

if (state.token) {
  api("/api/v1/health/summary")
    .then(() => enterApp())
    .catch(() => showLogin());
} else {
  showLogin();
}
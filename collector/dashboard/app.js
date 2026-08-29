const state = {
  ws: null,
  reconnectMs: 3000,
  summaryTimer: null,
  timeline: {},
};

function showLogin() {
  setToken("");
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

function onUnauthorizedPage() {
  if (!getToken() && $("#login")) showLogin();
}

onUnauthorized = onUnauthorizedPage;

function logout() {
  showLogin();
  $("#login-error").classList.add("hidden");
}

$("#login-form").addEventListener("submit", (e) => {
  e.preventDefault();
  const token = $("#token").value.trim();
  setToken(token);
  $("#login-error").classList.add("hidden");
  api("/api/v1/health/summary")
    .then(() => enterApp())
    .catch((err) => {
      setToken("");
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
            <td class="mono"><a class="host-link" href="host.html?agent=${encodeURIComponent(a.agent_id)}">${shortId(a.agent_id)}</a></td>
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
    <span class="detail">${describeEvent(ev)}</span>`;
  $("#timeline").prepend(li);
  while ($("#timeline").children.length > 100) {
    $("#timeline").lastChild.remove();
  }
}

function connectWs() {
  if (!getToken()) return;
  disconnectWs();
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const ws = new WebSocket(`${proto}://${location.host}/api/v1/events?token=${encodeURIComponent(getToken())}`);
  state.ws = ws;
  ws.onmessage = (msg) => {
    const ev = JSON.parse(msg.data);
    if (!document.querySelector("#view-overview").classList.contains("hidden")) {
      prependEvent(ev);
      resetSummaryTimer();
      if (ev.type === "connectivity_event") loadAgentsDebounced();
    }
  };
  ws.onclose = () => {
    state.ws = null;
    if (!getToken()) return;
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
    if (getToken() && !document.querySelector("#view-overview").classList.contains("hidden")) {
      loadAgents();
    }
  }, 10000);
}

window.addEventListener("hashchange", () => {
  const hash = location.hash || "#overview";
  switchView(hash);
});

if (getToken()) {
  api("/api/v1/health/summary")
    .then(() => enterApp())
    .catch(() => showLogin());
} else {
  showLogin();
}
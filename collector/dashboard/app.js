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
  if (hash === "#alerts") showAlertsTab(currentAtab);
}

let currentAtab = "rules";

function showAlertsTab(tab) {
  currentAtab = tab;
  document.querySelectorAll(".atab").forEach((s) => s.classList.add("hidden"));
  document.querySelectorAll(".tab").forEach((t) => t.classList.remove("active"));
  const section = $(`#atab-${tab}`);
  if (section) section.classList.remove("hidden");
  const btn = document.querySelector(`.tab[data-atab="${tab}"]`);
  if (btn) btn.classList.add("active");
  if (tab === "rules") loadRules();
  if (tab === "checks") loadChecks();
  if (tab === "active") loadActiveAlerts();
  if (tab === "ahistory") loadAlertHistory();
  if (tab === "timeline") loadTimelineView();
}

document.querySelectorAll(".tab").forEach((t) => {
  t.addEventListener("click", () => showAlertsTab(t.dataset.atab));
});

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

/* ===== Bloque 7.3: Alertas e historicos ===== */

function setFormMsg(id, text) {
  const el = $(id);
  if (!text) {
    el.classList.add("hidden");
    el.textContent = "";
    return;
  }
  el.classList.remove("ok");
  el.classList.add("error");
  el.textContent = text;
  el.classList.remove("hidden");
}

function showOk(el, text) {
  el.classList.remove("error");
  el.classList.add("ok");
  el.textContent = text;
  el.classList.remove("hidden");
}

function loadRules() {
  api("/api/v1/alerts/rules")
    .then((j) => {
      const body = $("#rules-body");
      body.innerHTML = (j.rules || []).length
        ? j.rules
            .map((r) => `<tr>
              <td class="mono">${r.id}</td>
              <td>${esc(r.name)}</td>
              <td class="mono">${esc(r.metric_name)}</td>
              <td class="mono">${r.entity ? esc(r.entity) : "&mdash;"}</td>
              <td>${opText(r.op)} ${fmtNum(r.threshold)}</td>
              <td>${r.for_secs}s</td>
              <td>${badge(r.enabled ? "on" : "off", r.enabled ? "online" : "unknown")}</td>
              <td><button class="btn ghost small" data-del-rule="${r.id}">borrar</button></td>
            </tr>`)
            .join("")
        : `<tr><td colspan="8" class="muted">sin reglas</td></tr>`;
      body.querySelectorAll("[data-del-rule]").forEach((b) => {
        b.addEventListener("click", () => deleteRule(b.dataset.delRule));
      });
    })
    .catch(() => {});
}

function deleteRule(id) {
  api(`/api/v1/alerts/rules/${id}`, { method: "DELETE" })
    .then(() => loadRules())
    .catch((e) => setFormMsg("#rule-msg", apiErrorText(e.message)));
}

$("#rule-form").addEventListener("submit", (e) => {
  e.preventDefault();
  const msg = $("#rule-msg");
  const body = {
    name: $("#rule-name").value.trim(),
    metric_name: $("#rule-metric").value.trim(),
    op: $("#rule-op").value,
    threshold: Number($("#rule-threshold").value),
    for_secs: Number($("#rule-for").value) || 0,
  };
  const entity = $("#rule-entity").value.trim();
  if (entity) body.entity = entity;
  api("/api/v1/alerts/rules", { method: "POST", body: JSON.stringify(body) })
    .then(() => {
      showOk(msg, "regla creada");
      $("#rule-name").value = "";
      $("#rule-metric").value = "";
      $("#rule-entity").value = "";
      $("#rule-threshold").value = "";
      loadRules();
    })
    .catch((err) => setFormMsg("#rule-msg", apiErrorText(err.message)));
});

function loadChecks() {
  api("/api/v1/health/checks")
    .then((j) => {
      const body = $("#checks-body");
      body.innerHTML = (j.checks || []).length
        ? j.checks
            .map((c) => `<tr>
              <td class="mono">${c.id}</td>
              <td>${esc(c.name)}</td>
              <td>${c.kind}</td>
              <td class="mono">${esc(c.target)}</td>
              <td>${c.interval_secs}s</td>
              <td>${badge(c.state || "unknown", c.state || "unknown")}</td>
              <td>${c.last_checked_at ? relTime(tsMs(c.last_checked_at)) : "nunca"} (${c.last_ok ? "ok" : "down"}, ${c.last_latency_ms ?? "?"}ms)</td>
              <td><button class="btn ghost small" data-results="${c.id}" data-name="${esc(c.name)}">ver</button></td>
              <td><button class="btn ghost small" data-del-check="${c.id}">borrar</button></td>
            </tr>`)
            .join("")
        : `<tr><td colspan="9" class="muted">sin checks</td></tr>`;
      body.querySelectorAll("[data-del-check]").forEach((b) => {
        b.addEventListener("click", () => deleteCheck(b.dataset.delCheck));
      });
      body.querySelectorAll("[data-results]").forEach((b) => {
        b.addEventListener("click", () => loadCheckResults(b.dataset.results, b.dataset.name));
      });
    })
    .catch(() => {});
}

function deleteCheck(id) {
  api(`/api/v1/health/checks/${id}`, { method: "DELETE" })
    .then(() => { loadChecks(); closeCheckResults(); })
    .catch((e) => setFormMsg("#check-msg", apiErrorText(e.message)));
}

function loadCheckResults(id, name) {
  api(`/api/v1/health/checks/${id}/results?limit=30`)
    .then((j) => {
      const box = $("#check-results");
      box.classList.remove("hidden");
      $("#check-results-title").textContent = `Resultados de ${name}`;
      $("#check-results-body").innerHTML = (j.results || []).length
        ? j.results
            .map((r) => `<tr>
              <td title="${r.ts}">${absTime(r.ts)}</td>
              <td>${badge(r.ok ? "ok" : "down", r.ok ? "up" : "down")}</td>
              <td>${r.latency_ms}ms</td>
              <td class="mono">${esc(r.detail)}</td>
            </tr>`)
            .join("")
        : `<tr><td colspan="4" class="muted">sin resultados</td></tr>`;
    })
    .catch(() => {});
}

function closeCheckResults() {
  $("#check-results").classList.add("hidden");
}

$("#check-form").addEventListener("submit", (e) => {
  e.preventDefault();
  const body = {
    name: $("#check-name").value.trim(),
    kind: $("#check-kind").value,
    target: $("#check-target").value.trim(),
    interval_secs: Number($("#check-interval").value),
  };
  const timeout = $("#check-timeout").value;
  if (timeout !== "") body.timeout_secs = Number(timeout);
  api("/api/v1/health/checks", { method: "POST", body: JSON.stringify(body) })
    .then(() => {
      showOk($("#check-msg"), "check creado");
      $("#check-name").value = "";
      $("#check-target").value = "";
      $("#check-interval").value = "";
      $("#check-timeout").value = "";
      loadChecks();
    })
    .catch((err) => setFormMsg("#check-msg", apiErrorText(err.message)));
});

function loadActiveAlerts() {
  let url = "/api/v1/alerts";
  const st = $("#active-state").value;
  if (st) url += `?state=${encodeURIComponent(st)}`;
  api(url)
    .then((j) => {
      const body = $("#active-body");
      body.innerHTML = (j.alerts || []).length
        ? j.alerts
            .map((a) => `<tr>
              <td>${esc(a.rule_name)}</td>
              <td class="mono">${esc(a.metric_name)}${a.entity ? ` [${esc(a.entity)}]` : ""}</td>
              <td>${opText(a.op)} ${fmtNum(a.threshold)}</td>
              <td class="mono"><a class="host-link" href="host.html?agent=${encodeURIComponent(a.agent_id)}">${shortId(a.agent_id)}</a></td>
              <td>${badge(a.state, a.state)}</td>
              <td title="${a.since}">${relTime(tsMs(a.since))}</td>
            </tr>`)
            .join("")
        : `<tr><td colspan="6" class="muted">sin alertas activas</td></tr>`;
    })
    .catch(() => {});
}

$("#active-state").addEventListener("change", loadActiveAlerts);
$("#reload-active").addEventListener("click", loadActiveAlerts);

function loadAlertHistory() {
  api("/api/v1/alerts/rules")
    .then((j) => {
      const sel = $("#ah-rule");
      if (sel.options.length <= 1) {
        (j.rules || []).forEach((r) => {
          const opt = document.createElement("option");
          opt.value = r.id;
          opt.textContent = `${r.id} · ${r.name}`;
          sel.appendChild(opt);
        });
      }
    })
    .catch(() => {});

  let url = "/api/v1/alerts/history?";
  const params = new URLSearchParams();
  const rule = $("#ah-rule").value;
  if (rule) params.set("rule_id", rule);
  const from = $("#ah-from").value;
  if (from !== "") params.set("from", from);
  const to = $("#ah-to").value;
  if (to !== "") params.set("to", to);
  params.set("limit", $("#ah-limit").value || 100);
  api(url + params.toString())
    .then((j) => {
      const body = $("#ahistory-body");
      body.innerHTML = (j.events || []).length
        ? j.events
            .map((e) => `<tr>
              <td title="${e.ts}">${absTime(e.ts)}</td>
              <td>${esc(e.rule_name)}</td>
              <td class="mono"><a class="host-link" href="host.html?agent=${encodeURIComponent(e.agent_id)}">${shortId(e.agent_id)}</a></td>
              <td>${badge(e.from_state || "inactive", e.from_state || "unknown")} &rarr; ${badge(e.to_state, e.to_state)}</td>
            </tr>`)
            .join("")
        : `<tr><td colspan="4" class="muted">sin eventos</td></tr>`;
    })
    .catch(() => {});
}

$("#reload-ahistory").addEventListener("click", loadAlertHistory);

function loadTimelineView() {
  let url = "/api/v1/events/history?";
  const params = new URLSearchParams();
  const agent = $("#tl-agent").value.trim();
  if (agent) params.set("agent_id", agent);
  params.set("limit", $("#tl-limit").value || 100);
  api(url + params.toString())
    .then((j) => {
      const type = $("#tl-type").value;
      const list = $("#tl-list");
      const events = (j.events || []).filter((ev) => !type || ev.type === type);
      list.innerHTML = events.length
        ? events
            .map((ev) => `<li>
              <span class="when" title="${ev.ts}">${absTime(ev.ts)}</span>
              <span class="kind tag-${ev.type}">${ev.type.replace("_", " ")}</span>
              <span class="detail">${describeEvent(ev)}</span>
            </li>`)
            .join("")
        : `<li class="muted">sin eventos</li>`;
    })
    .catch(() => {});
}

$("#reload-timeline").addEventListener("click", loadTimelineView);

if (getToken()) {
  api("/api/v1/health/summary")
    .then(() => enterApp())
    .catch(() => showLogin());
} else {
  showLogin();
}
const params = new URLSearchParams(location.search);
const AGENT_ID = (params.get("agent") || "").trim();
const state = {
  ws: null,
  reconnectMs: 3000,
  timeline: {},
  timer: null,
};

if (!AGENT_ID) {
  location.replace("index.html");
}

function boot() {
  onUnauthorized = () => location.replace("index.html#overview");
  $("#logout").addEventListener("click", () => {
    setToken("");
    location.replace("index.html");
  });
  $("#reload-series").addEventListener("click", reloadAll);
  document.querySelector("[data-chart-close]").addEventListener("click", closeChart);
  reloadAll();
  connectWs();
}

function ensureAgent() {
  if (!getToken()) {
    location.replace("index.html");
    return false;
  }
  return true;
}

function reloadAll() {
  loadHeader();
  loadAlerts();
  loadSeries();
  loadTimeline();
  loadReboots();
}

function loadHeader() {
  api(`/api/v1/agents/${encodeURIComponent(AGENT_ID)}`)
    .then((a) => {
      document.title = `Observer · Host ${shortId(a.agent_id)}`;
      $("#host-title").textContent = `Host ${shortId(a.agent_id)}`;
      $("#host-status").innerHTML = `${badge(a.state, a.state)} last seen ${relTime((a.last_seen_age_secs ?? 0) * 1000)}`;
      $("#agent-id").textContent = a.agent_id;
      $("#host-first").textContent = absTime(a.first_seen);
      $("#host-last").textContent = absTime(a.last_seen);
      $("#host-reboots").textContent = String(a.reboot_count ?? 0);
    })
    .catch(() => {});
}

function loadAlerts() {
  api(`/api/v1/alerts?agent_id=${encodeURIComponent(AGENT_ID)}`)
    .then((j) => {
      const ul = $("#alerts");
      ul.innerHTML = "";
      const empty = $("#alerts-empty");
      empty.classList.toggle("hidden", (j.alerts || []).length > 0);
      (j.alerts || []).forEach((al) => {
        const li = document.createElement("li");
        li.innerHTML = `
          <span>${badge(al.state, al.state)} ${esc(al.rule_name)}</span>
          <span class="muted">${esc(al.metric_name)} ${esc(opText(al.op))} ${fmtVal(al.threshold)}</span>
          <span class="when" title="${al.since}">desde ${relTime(tsMs(al.since))}</span>`;
        ul.appendChild(li);
      });
    })
    .catch(() => {});
}

function loadSeries() {
  api(`/api/v1/agents/${encodeURIComponent(AGENT_ID)}/metrics`)
    .then((j) => {
      const body = $("#series-body");
      if (!j.series) return;
      body.innerHTML = j.series
        .map((s) => {
          const ent = s.entity ? `${esc(s.entity)}` : "&mdash;";
          return `<tr>
            <td class="mono">${esc(s.metric_name)}</td>
            <td class="mono">${ent}</td>
            <td>${fmtVal(s.latest_value)}</td>
            <td>${s.samples}</td>
            <td><button class="btn ghost small" data-series="${esc(s.metric_name)}" data-entity="${esc(s.entity || "")}">graficar</button></td>
          </tr>`;
        })
        .join("");
      body.querySelectorAll("[data-series]").forEach((btn) => {
        btn.addEventListener("click", () => {
          showChart(btn.dataset.series, btn.dataset.entity);
        });
      });
    })
    .catch(() => {});
}

function showChart(metric, entity) {
  let url = `/api/v1/agents/${encodeURIComponent(AGENT_ID)}/metrics/${encodeURIComponent(metric)}?limit=300`;
  if (entity) url += `&entity=${encodeURIComponent(entity)}`;
  $("#chart-card").classList.remove("hidden");
  $("#chart-title").textContent = entity ? `${metric} [${entity}]` : metric;
  $("#chart").innerHTML = "";
  $("#chart-stats").textContent = "cargando...";
  api(url)
    .then((j) => {
      drawChart(j.points || []);
    })
    .catch(() => {
      $("#chart-stats").textContent = "sin datos para la serie";
    });
}

function closeChart() {
  $("#chart-card").classList.add("hidden");
  $("#chart").innerHTML = "";
  $("#chart-stats").textContent = "";
}

function drawChart(points) {
  const svg = $("#chart");
  svg.innerHTML = "";
  const stats = $("#chart-stats");
  if (!points.length) {
    stats.textContent = "sin puntos";
    return;
  }
  const W = 800;
  const H = 220;
  const PAD = 14;
  const vals = points.map((p) => p.value);
  let min = Math.min(...vals);
  let max = Math.max(...vals);
  if (max === min) {
    min -= 1;
    max += 1;
  }
  const n = Math.min(points.length, 300);
  const stride = Math.max(1, Math.ceil(points.length / 300));
  const t0 = tsMs(points[0].ts);
  const t1 = tsMs(points[points.length - 1].ts);
  const y = (v) => PAD + ((max - v) / (max - min)) * (H - 2 * PAD);
  const x = (i) => PAD + (i / Math.max(1, n - 1)) * (W - 2 * PAD);

  const idxs = [];
  for (let i = 0; i < points.length; i += stride) idxs.push(i);
  if (idxs[idxs.length - 1] !== points.length - 1) idxs.push(points.length - 1);
  const d = idxs.map((pi, drawn) => `${x(drawn)},${y(points[pi].value)}`).join(" ");

  svg.innerHTML = `
    <line x1="${PAD}" y1="${y(max)}" x2="${W - PAD}" y2="${y(max)}" class="axis"/>
    <polyline points="${d}" class="chart-line"/>
    <text x="${PAD + 4}" y="${y(max) - 4}" class="chart-label">${fmtVal(max)}</text>
    <text x="${PAD + 4}" y="${y(min) + 12}" class="chart-label">${fmtVal(min)}</text>
    <text x="${W - PAD}" y="${H - 4}" class="chart-label" text-anchor="end">${fmtRange(t0, t1)}</text>`;

  const avg = vals.reduce((a, b) => a + b, 0) / vals.length;
  stats.textContent = `ultimo ${fmtVal(vals[vals.length - 1])} · min ${fmtVal(min)} · max ${fmtVal(max)} · avg ${fmtVal(avg)} · ${points.length} muestras`;
}

function loadTimeline() {
  api(`/api/v1/events/history?agent_id=${encodeURIComponent(AGENT_ID)}&limit=50`)
    .then((j) => {
      state.timeline = {};
      const ul = $("#timeline");
      ul.innerHTML = "";
      (j.events || []).forEach((ev) => prependEvent(ev));
    })
    .catch(() => {});
}

function prependEvent(ev) {
  const key = `${ev.ts}-${ev.check_id || ev.rule_id || ev.reboot_ref || ev.id || ""}`;
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

function loadReboots() {
  api(`/api/v1/agents/${encodeURIComponent(AGENT_ID)}/reboots?limit=20`)
    .then((j) => {
      const ul = $("#reboots");
      if (!j.reboots) return;
      ul.innerHTML = j.reboots.length
        ? j.reboots
            .map((r) => `<li>
            <span class="when" title="${r.detected_at}">${relTime(tsMs(r.detected_at))}</span>
            <span class="kind tag-reboot_event">reboot</span>
            <span class="detail">uptime ${fmtUptime(r.uptime_before)} \u2192 ${fmtUptime(r.uptime_after)}</span>
          </li>`)
            .join("")
        : `<li class="muted">sin reboots registrados</li>`;
    })
    .catch(() => {});
}

function connectWs() {
  if (!getToken()) return;
  disconnectWs();
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const ws = new WebSocket(`${proto}://${location.host}/api/v1/events?token=${encodeURIComponent(getToken())}`);
  state.ws = ws;
  ws.onmessage = (msg) => {
    const ev = JSON.parse(msg.data);
    if (ev.type === "events_lagged") return;
    if (ev.agent_id && ev.agent_id !== AGENT_ID) return;
    prependEvent(ev);
    scheduleRefresh();
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

function scheduleRefresh() {
  if (state.timer) clearTimeout(state.timer);
  state.timer = setTimeout(() => {
    state.timer = null;
    if (ensureAgent()) {
      loadHeader();
      loadAlerts();
      loadSeries();
      loadReboots();
    }
  }, 900);
}

function fmtVal(v) {
  if (v === null || v === undefined) return "-";
  if (Number.isInteger(v)) return String(v);
  return Number(v).toFixed(2);
}

function fmtRange(ms0, ms1) {
  const a = new Date(ms0).toLocaleTimeString();
  const b = ms1 === ms0 ? a : new Date(ms1).toLocaleTimeString();
  return `${a} - ${b}`;
}

if (getToken() && AGENT_ID) {
  boot();
} else {
  location.replace("index.html");
}
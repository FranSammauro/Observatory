const TOKEN_KEY = "obs_token";
const $ = (sel) => document.querySelector(sel);
let onUnauthorized = null;

function getToken() {
  return sessionStorage.getItem(TOKEN_KEY) || "";
}

function setToken(token) {
  token ? sessionStorage.setItem(TOKEN_KEY, token) : sessionStorage.removeItem(TOKEN_KEY);
}

function api(path) {
  return fetch(path, {
    headers: { Authorization: `Bearer ${getToken()}` },
  }).then(async (res) => {
    if (res.status === 401) {
      setToken("");
      if (onUnauthorized) onUnauthorized();
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
  if (sec === null || sec === undefined) return "?";
  const days = Math.floor(sec / 86400);
  const h = Math.floor((sec % 86400) / 3600);
  if (days > 0) return `${days}d ${h}h`;
  const m = Math.floor((sec % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  return `${Math.floor(sec)}s`;
}

function badge(text, cls) {
  return `<span class="badge ${cls}">${text}</span>`;
}

function esc(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => (
    { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]
  ));
}

function opText(op) {
  return { gt: ">", gte: ">=", lt: "<", lte: "<=", eq: "=", ne: "!=" }[op] || op;
}

function fmtNum(v) {
  if (v === null || v === undefined) return "-";
  if (Number.isInteger(v)) return String(v);
  return Number(v).toFixed(2);
}

function apiErrorText(code) {
  const map = {
    invalid_rule_name: "nombre invalido",
    invalid_metric_name: "metrica invalida",
    invalid_entity: "entidad invalida",
    invalid_op: "operador invalido (ge, gt, le o lt)",
    invalid_threshold: "umbral invalido",
    invalid_for_secs: "durante no puede ser negativo",
    rule_already_exists: "ya existe una regla con ese nombre",
    unknown_rule: "regla no encontrada",
    invalid_rule_id: "rule_id invalido",
    invalid_check: "check invalido",
    invalid_check_kind: "kind debe ser http o tcp",
    invalid_check_interval: "intervalo entre 1 y 86400",
    invalid_check_timeout: "timeout entre 1 y 300",
    invalid_check_target: "target invalido",
    check_already_exists: "ya existe un check con ese nombre",
    unknown_check: "check no encontrado",
    invalid_check_id: "check_id invalido",
    invalid_alert_state: "estado debe ser pending o firing",
    invalid_agent_id: "agent_id no es un UUID valido",
    invalid_time_range: "desde no puede ser posterior a hasta",
  };
  return map[code] || (code ? `error ${code}` : "error");
}

function describeEvent(ev) {
  switch (ev.type) {
    case "alert_event":
      return `${ev.rule_name} ${ev.from_state || "inactive"} \u2192 ${ev.to_state} ` +
        `(agent ${shortId(ev.agent_id)})`;
    case "health_result":
      return `${ev.check_name}: ${ev.ok ? "ok" : "down"} ` +
        `(${ev.detail}) ${ev.state_changed ? `· estado ${ev.state}` : ""}`;
    case "reboot_event":
      return `reboot detectado (uptime ${fmtUptime(ev.uptime_before)} \u2192 ${fmtUptime(ev.uptime_after)})`;
    case "connectivity_event":
      return `conectividad ${ev.from_state || "desconocido"} \u2192 ${ev.to_state}`;
    case "events_lagged":
      return `cliente atrasado: se descartaron ${ev.dropped} eventos`;
    default:
      return JSON.stringify(ev);
  }
}
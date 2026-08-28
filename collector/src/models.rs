use serde::Deserialize;
use std::collections::HashMap;

use crate::config::{MAX_ARRAY_ENTRIES, MAX_METRIC_KEYS, PROTOCOL_VERSION};
use crate::error::ApiError;

/*
 * Payloads que el agent envia (ver agent/src/protocol.c):
 *   - POST /api/v1/metrics           -> MetricsPayload
 *   - POST /api/v1/agents/heartbeat  -> HeartbeatPayload
 *
 * El agent serializa con un escritor JSON manual (agent/src/protocol.c);
 * los nombres de campo de abajo deben coincidir 1:1 con los que el agent
 * emite. `metrics` es un objeto plano nombre->valor (escalares); los
 * arrays disk/network/filesystem llevan entidades con nombre.
 */

#[derive(Debug, Deserialize)]
pub struct MetricsPayload {
    #[serde(rename = "protocol_version")]
    pub protocol_version: i32,
    #[serde(rename = "agent_id")]
    pub agent_id: String,
    pub timestamp: u64,
    #[serde(default)]
    pub metrics: HashMap<String, f64>,
    #[serde(default)]
    pub disk: Vec<DiskEntry>,
    #[serde(default)]
    pub network: Vec<NetworkEntry>,
    #[serde(default)]
    pub filesystem: Vec<FilesystemEntry>,
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatPayload {
    #[serde(rename = "protocol_version")]
    pub protocol_version: i32,
    #[serde(rename = "agent_id")]
    pub agent_id: String,
    pub timestamp: u64,
}

#[derive(Debug, Deserialize)]
pub struct DiskEntry {
    pub device: String,
    #[serde(rename = "read_bytes_per_sec")]
    pub read_bytes_per_sec: f64,
    #[serde(rename = "write_bytes_per_sec")]
    pub write_bytes_per_sec: f64,
    #[serde(rename = "read_ops_per_sec")]
    pub read_ops_per_sec: f64,
    #[serde(rename = "write_ops_per_sec")]
    pub write_ops_per_sec: f64,
}

#[derive(Debug, Deserialize)]
pub struct NetworkEntry {
    pub interface: String,
    #[serde(rename = "rx_bytes_per_sec")]
    pub rx_bytes_per_sec: f64,
    #[serde(rename = "tx_bytes_per_sec")]
    pub tx_bytes_per_sec: f64,
    #[serde(rename = "rx_packets_per_sec")]
    pub rx_packets_per_sec: f64,
    #[serde(rename = "tx_packets_per_sec")]
    pub tx_packets_per_sec: f64,
    #[serde(rename = "rx_errors_total")]
    pub rx_errors_total: u64,
    #[serde(rename = "tx_errors_total")]
    pub tx_errors_total: u64,
}

#[derive(Debug, Deserialize)]
pub struct FilesystemEntry {
    pub device: String,
    pub mountpoint: String,
    #[serde(rename = "fs_type")]
    pub fs_type: String,
    #[serde(rename = "total_bytes")]
    pub total_bytes: u64,
    #[serde(rename = "available_bytes")]
    pub available_bytes: u64,
    pub utilization: f64,
}

impl MetricsPayload {
    /*
     * Aplana el payload en el modelo de la tabla `metric_samples`: una
     * fila por (metric_name, entity, value). Los escalares de `metrics`
     * quedan con entity = NULL; cada entrada de los arrays se expande en
     * `metric_name` = "<categoria>.<campo>" con entity = device /
     * mountpoint / interface.
     */
    pub fn to_metric_rows(&self) -> Vec<(String, Option<String>, f64)> {
        let mut rows = Vec::with_capacity(
            self.metrics.len()
                + self.disk.len() * 4
                + self.network.len() * 6
                + self.filesystem.len() * 3,
        );

        for (name, value) in &self.metrics {
            rows.push((name.clone(), None, *value));
        }

        for d in &self.disk {
            rows.push((
                "disk.read_bytes_per_sec".into(),
                Some(d.device.clone()),
                d.read_bytes_per_sec,
            ));
            rows.push((
                "disk.write_bytes_per_sec".into(),
                Some(d.device.clone()),
                d.write_bytes_per_sec,
            ));
            rows.push((
                "disk.read_ops_per_sec".into(),
                Some(d.device.clone()),
                d.read_ops_per_sec,
            ));
            rows.push((
                "disk.write_ops_per_sec".into(),
                Some(d.device.clone()),
                d.write_ops_per_sec,
            ));
        }

        for n in &self.network {
            rows.push((
                "network.rx_bytes_per_sec".into(),
                Some(n.interface.clone()),
                n.rx_bytes_per_sec,
            ));
            rows.push((
                "network.tx_bytes_per_sec".into(),
                Some(n.interface.clone()),
                n.tx_bytes_per_sec,
            ));
            rows.push((
                "network.rx_packets_per_sec".into(),
                Some(n.interface.clone()),
                n.rx_packets_per_sec,
            ));
            rows.push((
                "network.tx_packets_per_sec".into(),
                Some(n.interface.clone()),
                n.tx_packets_per_sec,
            ));
            rows.push((
                "network.rx_errors_total".into(),
                Some(n.interface.clone()),
                n.rx_errors_total as f64,
            ));
            rows.push((
                "network.tx_errors_total".into(),
                Some(n.interface.clone()),
                n.tx_errors_total as f64,
            ));
        }

        for f in &self.filesystem {
            rows.push((
                "filesystem.total_bytes".into(),
                Some(f.mountpoint.clone()),
                f.total_bytes as f64,
            ));
            rows.push((
                "filesystem.available_bytes".into(),
                Some(f.mountpoint.clone()),
                f.available_bytes as f64,
            ));
            rows.push((
                "filesystem.utilization".into(),
                Some(f.mountpoint.clone()),
                f.utilization,
            ));
        }

        rows
    }
}

pub trait Validate {
    fn validate(&self) -> Result<(), ApiError>;
}

fn check_protocol_version(v: i32) -> Result<(), ApiError> {
    if v != PROTOCOL_VERSION {
        return Err(ApiError::bad_request(
            "unsupported_protocol_version",
            format!("protocol_version {v} no soportado (se espera {PROTOCOL_VERSION})"),
        ));
    }
    Ok(())
}

fn check_agent_id(id: &str) -> Result<(), ApiError> {
    if uuid::Uuid::parse_str(id.trim()).is_err() {
        return Err(ApiError::bad_request(
            "invalid_agent_id",
            format!("agent_id '{id}' no es un UUID valido"),
        ));
    }
    Ok(())
}

fn check_array_len(name: &str, len: usize) -> Result<(), ApiError> {
    if len > MAX_ARRAY_ENTRIES {
        return Err(ApiError::bad_request(
            "too_many_entities",
            format!("{name} tiene {len} entradas (max {MAX_ARRAY_ENTRIES})"),
        ));
    }
    Ok(())
}

fn check_non_empty(field: &str, value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::bad_request(
            "invalid_entity",
            format!("{field} no puede estar vacio"),
        ));
    }
    Ok(())
}

impl Validate for MetricsPayload {
    fn validate(&self) -> Result<(), ApiError> {
        check_protocol_version(self.protocol_version)?;
        check_agent_id(&self.agent_id)?;
        if self.metrics.len() > MAX_METRIC_KEYS {
            return Err(ApiError::bad_request(
                "too_many_metrics",
                format!(
                    "metrics tiene {} claves (max {MAX_METRIC_KEYS})",
                    self.metrics.len()
                ),
            ));
        }
        check_array_len("disk", self.disk.len())?;
        check_array_len("network", self.network.len())?;
        check_array_len("filesystem", self.filesystem.len())?;

        for d in &self.disk {
            check_non_empty("disk.device", &d.device)?;
        }
        for n in &self.network {
            check_non_empty("network.interface", &n.interface)?;
        }
        for f in &self.filesystem {
            check_non_empty("filesystem.device", &f.device)?;
            check_non_empty("filesystem.mountpoint", &f.mountpoint)?;
            check_non_empty("filesystem.fs_type", &f.fs_type)?;
        }
        Ok(())
    }
}

impl Validate for HeartbeatPayload {
    fn validate(&self) -> Result<(), ApiError> {
        check_protocol_version(self.protocol_version)?;
        check_agent_id(&self.agent_id)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "protocol_version": 1,
        "agent_id": "bca99f718eaaf6f155b214a92fdd309f",
        "timestamp": 1720000000,
        "metrics": {
            "system.cpu.utilization": 0.1234,
            "system.memory.total": 8589934592,
            "system.uptime": 123456
        },
        "disk": [
            {"device": "sda", "read_bytes_per_sec": 1.5, "write_bytes_per_sec": 2.5,
             "read_ops_per_sec": 0.1, "write_ops_per_sec": 0.2}
        ],
        "network": [
            {"interface": "eth0", "rx_bytes_per_sec": 100.0, "tx_bytes_per_sec": 50.0,
             "rx_packets_per_sec": 10.0, "tx_packets_per_sec": 5.0,
             "rx_errors_total": 0, "tx_errors_total": 0}
        ],
        "filesystem": [
            {"device": "/dev/sda2", "mountpoint": "/", "fs_type": "ext4",
             "total_bytes": 1024, "available_bytes": 512, "utilization": 0.5}
        ]
    }"#;

    #[test]
    fn parses_agent_sample() {
        let p: MetricsPayload = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(p.protocol_version, 1);
        assert_eq!(p.metrics.len(), 3);
        assert_eq!(p.disk.len(), 1);
        assert_eq!(p.network.len(), 1);
        assert_eq!(p.filesystem.len(), 1);
    }

    #[test]
    fn flattens_to_expected_row_count() {
        let p: MetricsPayload = serde_json::from_str(SAMPLE).unwrap();
        let rows = p.to_metric_rows();
        assert_eq!(rows.len(), 3 + 4 + 6 + 3);
    }

    #[test]
    fn rejects_wrong_protocol_version() {
        let mut p: MetricsPayload = serde_json::from_str(SAMPLE).unwrap();
        p.protocol_version = 2;
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_bad_agent_id() {
        let mut p: MetricsPayload = serde_json::from_str(SAMPLE).unwrap();
        p.agent_id = "not-a-uuid".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_oversized_arrays() {
        let mut p: MetricsPayload = serde_json::from_str(SAMPLE).unwrap();
        p.disk = (0..MAX_ARRAY_ENTRIES + 1)
            .map(|i| DiskEntry {
                device: format!("sdd{i}"),
                read_bytes_per_sec: 0.0,
                write_bytes_per_sec: 0.0,
                read_ops_per_sec: 0.0,
                write_ops_per_sec: 0.0,
            })
            .collect();
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_empty_entity() {
        let mut p: MetricsPayload = serde_json::from_str(SAMPLE).unwrap();
        p.network[0].interface = "".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_too_many_metric_keys() {
        let mut p: MetricsPayload = serde_json::from_str(SAMPLE).unwrap();
        p.metrics = (0..MAX_METRIC_KEYS + 1)
            .map(|i| (format!("m.{i}"), 1.0))
            .collect();
        assert!(p.validate().is_err());
    }
}

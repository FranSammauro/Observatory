/*
 * Deteccion de reboot (Fase 4, bloque 4.3).
 *
 * `system.uptime` es monotono (segundos desde el boot del host, medidos
 * por el agent en /proc/uptime). SIEMPRE sube entre muestras de un mismo
 * host; si una muestra llega con un valor menor al ultimo conocido hubo
 * un reboot. En ingestion se compara contra el ultimo uptime almacenado
 * del agent y, si cayo mas de la tolerancia, se registra un
 * `reboot_events`.
 *
 * Tolerancia: el uptime se serializa como double y el segundo de
 * /proc/uptime es continuo; un redondeo en el borde de segundos puede
 * dar una caida aparente de ~1s. La caida minima configurable
 * (OBS_REBOOT_MIN_UPTIME_DROP_SECS, default 2s) filtra ese ruido sin
 * perder un reboot real (que siempre cae a segundos desde cero).
 */

pub fn detect_reboot(
    previous_uptime: Option<f64>,
    current_uptime: f64,
    min_drop_secs: f64,
) -> bool {
    match previous_uptime {
        Some(prev) => {
            prev.is_finite()
                && current_uptime.is_finite()
                && min_drop_secs.is_finite()
                && min_drop_secs >= 0.0
                && prev - current_uptime > min_drop_secs
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_previous_sample_is_not_a_reboot() {
        assert!(!detect_reboot(None, 42.0, 2.0));
    }

    #[test]
    fn increasing_uptime_is_not_a_reboot() {
        assert!(!detect_reboot(Some(42.0), 43.5, 2.0));
    }

    #[test]
    fn equal_uptime_is_not_a_reboot() {
        assert!(!detect_reboot(Some(42.0), 42.0, 2.0));
    }

    #[test]
    fn small_drop_within_tolerance_is_not_a_reboot() {
        assert!(!detect_reboot(Some(42.5), 42.2, 2.0));
    }

    #[test]
    fn drop_equal_to_tolerance_is_not_a_reboot() {
        assert!(!detect_reboot(Some(44.0), 42.0, 2.0));
    }

    #[test]
    fn large_drop_is_a_reboot() {
        assert!(detect_reboot(Some(3600.0), 1.5, 2.0));
    }

    #[test]
    fn zero_tolerance_flags_any_strict_drop() {
        assert!(!detect_reboot(Some(42.0), 42.0, 0.0));
        assert!(detect_reboot(Some(42.0), 41.9, 0.0));
    }

    #[test]
    fn non_finite_values_are_ignored() {
        assert!(!detect_reboot(Some(f64::NAN), 42.0, 2.0));
        assert!(!detect_reboot(Some(42.0), f64::INFINITY, 2.0));
        assert!(!detect_reboot(Some(42.0), 1.0, f64::NAN));
        assert!(!detect_reboot(Some(42.0), 1.0, -1.0));
    }
}

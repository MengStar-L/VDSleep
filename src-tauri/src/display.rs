use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn log_display_error(message: &str) {
    eprintln!("[Display] {message}");
}

#[derive(Debug, Clone, Serialize)]
pub struct MonitorInfo {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub monitor_type: String,
    pub active: bool,
    pub primary: bool,
    pub bounds: String,
    pub hardware_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DisplayLayoutSnapshot {
    displays: Vec<DisplayModeSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DisplayModeSnapshot {
    id: String,
    device_name: String,
    device_string: String,
    hardware_id: String,
    active: bool,
    primary: bool,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    bits_per_pel: u32,
    frequency: u32,
    orientation: u32,
    fixed_output: u32,
}

pub struct DisplayController {
    cooldown_until: Mutex<Option<Instant>>,
    initial_config_path: PathBuf,
    initial_state_captured: AtomicBool,
    switched: AtomicBool,
}

impl DisplayController {
    pub fn new() -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let config_dir = exe_dir.join("config");
        let _ = std::fs::create_dir_all(&config_dir);

        Self {
            cooldown_until: Mutex::new(None),
            initial_config_path: config_dir.join("display_initial_state.json"),
            initial_state_captured: AtomicBool::new(false),
            switched: AtomicBool::new(false),
        }
    }

    pub fn is_switched(&self) -> bool {
        self.switched.load(Ordering::Relaxed)
    }

    pub fn has_initial_state(&self) -> bool {
        self.initial_state_captured.load(Ordering::Relaxed) && self.initial_config_path.exists()
    }

    pub fn is_vdd_installed(&self) -> bool {
        self.get_all_monitors()
            .iter()
            .any(|monitor| monitor.monitor_type == "virtual")
    }

    pub fn capture_initial_state(&self) -> bool {
        if self.is_switched() {
            return false;
        }

        match native::capture_layout() {
            Ok(snapshot) => match serde_json::to_string_pretty(&snapshot) {
                Ok(json) => {
                    if std::fs::write(&self.initial_config_path, json).is_ok() {
                        self.initial_state_captured.store(true, Ordering::Relaxed);
                        true
                    } else {
                        log_display_error("failed to save startup display state");
                        false
                    }
                }
                Err(error) => {
                    log_display_error(&format!(
                        "failed to serialize startup display state: {error}"
                    ));
                    false
                }
            },
            Err(error) => {
                log_display_error(&format!("failed to capture startup display state: {error}"));
                false
            }
        }
    }

    pub fn capture_initial_state_if_needed(&self) -> bool {
        if self.has_initial_state() {
            return true;
        }
        self.capture_initial_state()
    }

    pub fn get_all_monitors(&self) -> Vec<MonitorInfo> {
        match native::enumerate_monitors() {
            Ok(monitors) if !monitors.is_empty() => monitors,
            Ok(_) => fallback_monitors(),
            Err(error) => {
                log_display_error(&format!("failed to enumerate monitors: {error}"));
                fallback_monitors()
            }
        }
    }

    pub fn switch_to_display(&self, display_id: &str) -> bool {
        if display_id.trim().is_empty() {
            return false;
        }

        let in_cooldown = self
            .cooldown_until
            .lock()
            .map(|until| Instant::now() < until)
            .unwrap_or(false);
        if in_cooldown {
            return false;
        }

        if !self.capture_initial_state_if_needed() {
            log_display_error("startup display state is not ready; skipping switch");
            return false;
        }

        match native::switch_to_display(display_id) {
            Ok(_) => {
                self.switched.store(true, Ordering::Relaxed);
                true
            }
            Err(error) => {
                log_display_error(&format!("failed to switch display: {error}"));
                false
            }
        }
    }

    pub fn restore_display(&self) -> bool {
        if !self.has_initial_state() {
            return false;
        }

        let snapshot = match std::fs::read_to_string(&self.initial_config_path)
            .ok()
            .and_then(|content| serde_json::from_str::<DisplayLayoutSnapshot>(&content).ok())
        {
            Some(snapshot) => snapshot,
            None => {
                self.initial_state_captured.store(false, Ordering::Relaxed);
                log_display_error("failed to read startup display state");
                return false;
            }
        };

        match native::restore_layout(&snapshot) {
            Ok(_) => {
                self.switched.store(false, Ordering::Relaxed);
                *self.cooldown_until.lock() = Some(Instant::now() + Duration::from_secs(3));
                true
            }
            Err(error) => {
                log_display_error(&format!("failed to restore startup display state: {error}"));
                false
            }
        }
    }
}

fn fallback_monitors() -> Vec<MonitorInfo> {
    vec![MonitorInfo {
        id: "\\\\.\\DISPLAY1".to_string(),
        name: "Primary display".to_string(),
        monitor_type: "physical".to_string(),
        active: true,
        primary: true,
        bounds: "Unknown".to_string(),
        hardware_id: String::new(),
    }]
}

#[cfg(windows)]
mod native {
    use super::{DisplayLayoutSnapshot, DisplayModeSnapshot, MonitorInfo};
    use std::collections::{HashMap, HashSet};
    use std::mem;
    use windows::core::PCWSTR;
    use windows::Win32::Devices::Display::{
        DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
        DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO,
        DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_VIRTUAL,
        DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
        DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ALL_PATHS, QDC_VIRTUAL_MODE_AWARE,
        QDC_VIRTUAL_REFRESH_RATE_AWARE,
    };
    use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, LUID, POINTL};
    use windows::Win32::Graphics::Gdi::{
        ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsExW, CDS_NORESET,
        CDS_TYPE, CDS_UPDATEREGISTRY, DEVMODEW, DEVMODEW_0_1, DEVMODE_DISPLAY_FIXED_OUTPUT,
        DEVMODE_DISPLAY_ORIENTATION, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP,
        DISPLAY_DEVICE_PRIMARY_DEVICE, DISP_CHANGE_SUCCESSFUL, DM_BITSPERPEL,
        DM_DISPLAYFIXEDOUTPUT, DM_DISPLAYFREQUENCY, DM_DISPLAYORIENTATION, DM_PELSHEIGHT,
        DM_PELSWIDTH, DM_POSITION, EDS_RAWMODE, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS,
        ENUM_DISPLAY_SETTINGS_MODE, ENUM_REGISTRY_SETTINGS,
    };

    const DISPLAYCONFIG_PATH_ACTIVE: u32 = 0x0000_0001;

    #[derive(Clone)]
    struct RuntimeMonitor {
        info: MonitorInfo,
        mode: Option<DisplayModeSnapshot>,
        ccd_rank: usize,
    }

    #[derive(Clone)]
    struct GdiDisplay {
        device_name: String,
        adapter_name: String,
        monitor_name: String,
        monitor_id: String,
        active: bool,
        primary: bool,
        mode: Option<DisplayModeSnapshot>,
    }

    pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>, String> {
        Ok(enumerate_runtime_monitors()?
            .into_iter()
            .map(|monitor| monitor.info)
            .collect())
    }

    pub fn capture_layout() -> Result<DisplayLayoutSnapshot, String> {
        let displays = enumerate_gdi_displays()
            .into_iter()
            .filter_map(|display| display.mode)
            .collect::<Vec<_>>();

        if displays.iter().any(|display| display.active) {
            Ok(DisplayLayoutSnapshot { displays })
        } else {
            Err("no active display can be recorded".to_string())
        }
    }

    pub fn switch_to_display(display_id: &str) -> Result<(), String> {
        let monitors = enumerate_runtime_monitors()?;
        let target = monitors
            .iter()
            .find(|monitor| monitor.info.id == display_id)
            .or_else(|| {
                monitors.iter().find(|monitor| {
                    monitor.info.hardware_id == display_id
                        || monitor
                            .mode
                            .as_ref()
                            .map(|mode| mode.device_name == display_id)
                            .unwrap_or(false)
                })
            })
            .cloned()
            .ok_or_else(|| format!("target display not found: {display_id}"))?;

        let mut target_mode = target
            .mode
            .clone()
            .ok_or_else(|| format!("target display cannot be controlled: {}", target.info.name))?;

        target_mode.active = true;
        target_mode.primary = true;
        target_mode.x = 0;
        target_mode.y = 0;
        apply_mode(&target_mode, true)?;

        let mut detached = HashSet::new();
        for monitor in monitors {
            if monitor.info.id == target.info.id || !monitor.info.active {
                continue;
            }
            let Some(mode) = monitor.mode.as_ref() else {
                continue;
            };
            if detached.insert(mode.device_name.clone()) {
                detach_display(&mode.device_name, &monitor.info.name)?;
            }
        }

        apply_pending_changes()
    }

    pub fn restore_layout(snapshot: &DisplayLayoutSnapshot) -> Result<(), String> {
        if !snapshot.displays.iter().any(|display| display.active) {
            return Err("startup display state has no active display".to_string());
        }

        for display in snapshot.displays.iter().filter(|display| display.active) {
            apply_mode(display, true)?;
        }

        for display in snapshot.displays.iter().filter(|display| !display.active) {
            detach_display(&display.device_name, &display.device_string)?;
        }

        apply_pending_changes()
    }

    fn enumerate_runtime_monitors() -> Result<Vec<RuntimeMonitor>, String> {
        let gdi_displays = enumerate_gdi_displays();
        let mut monitors = enumerate_ccd_monitors(&gdi_displays)?;

        if monitors.is_empty() {
            monitors = gdi_displays
                .into_iter()
                .enumerate()
                .map(|(index, display)| monitor_from_gdi(index, display))
                .collect();
        }

        monitors.sort_by(|a, b| {
            b.info
                .active
                .cmp(&a.info.active)
                .then_with(|| b.info.primary.cmp(&a.info.primary))
                .then_with(|| a.ccd_rank.cmp(&b.ccd_rank))
                .then_with(|| a.info.name.cmp(&b.info.name))
        });

        Ok(monitors)
    }

    fn enumerate_ccd_monitors(gdi_displays: &[GdiDisplay]) -> Result<Vec<RuntimeMonitor>, String> {
        let (paths, modes) = query_display_config_all_paths()?;
        let source_modes = source_mode_lookup(&modes);
        let gdi_by_name = gdi_displays
            .iter()
            .map(|display| (display.device_name.to_uppercase(), display.clone()))
            .collect::<HashMap<_, _>>();
        let mut monitors_by_target = HashMap::new();

        for (rank, path) in paths.iter().enumerate() {
            let active = path.flags & DISPLAYCONFIG_PATH_ACTIVE != 0;
            let target_name = target_device_name(path).unwrap_or_default();
            let source_name = if active {
                source_device_name(path).unwrap_or_default()
            } else {
                String::new()
            };
            let monitor_path = wide_to_string(&target_name.monitorDevicePath);
            let friendly = wide_to_string(&target_name.monitorFriendlyDeviceName);
            if !active && monitor_path.is_empty() {
                continue;
            }

            let device_name = if !source_name.is_empty() {
                source_name
            } else {
                match find_gdi_for_target(&monitor_path, &friendly, gdi_displays) {
                    Some(display) => display.device_name,
                    None => String::new(),
                }
            };

            let key = if !monitor_path.is_empty() {
                normalize_id(&monitor_path)
            } else {
                format!(
                    "ccd:{}:{}:{}",
                    path.targetInfo.adapterId.HighPart,
                    path.targetInfo.adapterId.LowPart,
                    path.targetInfo.id
                )
            };

            let gdi = if device_name.is_empty() {
                None
            } else {
                gdi_by_name.get(&device_name.to_uppercase()).cloned()
            };
            let source_mode =
                source_modes.get(&(luid_key(path.sourceInfo.adapterId), path.sourceInfo.id));
            let mode = make_mode_snapshot(
                &key,
                &device_name,
                &friendly,
                &monitor_path,
                active,
                gdi.as_ref(),
                source_mode,
            );
            let primary = active && gdi.as_ref().map(|display| display.primary).unwrap_or(false);
            let output_technology = target_name.outputTechnology;
            let monitor_type = if output_technology
                == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_VIRTUAL
                || is_virtual_display(&friendly, &monitor_path, &device_name)
            {
                "virtual"
            } else {
                "physical"
            };
            let display_label = monitor_display_name(&friendly, &monitor_path, gdi.as_ref(), rank);
            let bounds = mode
                .as_ref()
                .filter(|_| active)
                .map(|mode| format!("{}x{} @ ({}, {})", mode.width, mode.height, mode.x, mode.y))
                .unwrap_or_else(|| "未启用".to_string());

            let monitor = RuntimeMonitor {
                info: MonitorInfo {
                    id: key.clone(),
                    name: if monitor_type == "virtual" {
                        format!("虚拟显示器 {display_label}")
                    } else {
                        format!("物理显示器 {display_label}")
                    },
                    monitor_type: monitor_type.to_string(),
                    active,
                    primary,
                    bounds,
                    hardware_id: monitor_path,
                },
                mode,
                ccd_rank: rank,
            };

            if monitors_by_target
                .get(&key)
                .map(|current| should_replace_monitor(current, &monitor))
                .unwrap_or(true)
            {
                monitors_by_target.insert(key, monitor);
            }
        }

        Ok(monitors_by_target.into_values().collect())
    }

    fn should_replace_monitor(current: &RuntimeMonitor, candidate: &RuntimeMonitor) -> bool {
        monitor_score(candidate) > monitor_score(current)
            || (monitor_score(candidate) == monitor_score(current)
                && candidate.ccd_rank < current.ccd_rank)
    }

    fn monitor_score(monitor: &RuntimeMonitor) -> u8 {
        let mut score = 0;
        if monitor.info.active {
            score += 4;
        }
        if monitor.info.primary {
            score += 2;
        }
        if monitor.mode.is_some() {
            score += 1;
        }
        score
    }

    fn monitor_from_gdi(index: usize, display: GdiDisplay) -> RuntimeMonitor {
        let id = if display.monitor_id.is_empty() {
            display.device_name.clone()
        } else {
            normalize_id(&display.monitor_id)
        };
        let monitor_type = if is_virtual_display(
            &display.monitor_name,
            &display.monitor_id,
            &display.adapter_name,
        ) {
            "virtual"
        } else {
            "physical"
        };
        let label = if display.monitor_name.is_empty() {
            display.adapter_name.clone()
        } else {
            display.monitor_name.clone()
        };
        let bounds = display
            .mode
            .as_ref()
            .filter(|_| display.active)
            .map(|mode| format!("{}x{} @ ({}, {})", mode.width, mode.height, mode.x, mode.y))
            .unwrap_or_else(|| "未启用".to_string());

        RuntimeMonitor {
            info: MonitorInfo {
                id,
                name: if monitor_type == "virtual" {
                    format!("虚拟显示器 {label}")
                } else {
                    format!("物理显示器 {label}")
                },
                monitor_type: monitor_type.to_string(),
                active: display.active,
                primary: display.primary,
                bounds,
                hardware_id: display.monitor_id,
            },
            mode: display.mode,
            ccd_rank: index,
        }
    }

    fn query_display_config_all_paths(
    ) -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>), String> {
        let flags = QDC_ALL_PATHS | QDC_VIRTUAL_MODE_AWARE | QDC_VIRTUAL_REFRESH_RATE_AWARE;

        for _ in 0..3 {
            let mut path_count = 0;
            let mut mode_count = 0;
            let result =
                unsafe { GetDisplayConfigBufferSizes(flags, &mut path_count, &mut mode_count) };
            if result != ERROR_SUCCESS {
                return Err(format!("GetDisplayConfigBufferSizes failed: {}", result.0));
            }

            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
            let result = unsafe {
                QueryDisplayConfig(
                    flags,
                    &mut path_count,
                    paths.as_mut_ptr(),
                    &mut mode_count,
                    modes.as_mut_ptr(),
                    None,
                )
            };

            if result == ERROR_SUCCESS {
                paths.truncate(path_count as usize);
                modes.truncate(mode_count as usize);
                return Ok((paths, modes));
            }

            if result != ERROR_INSUFFICIENT_BUFFER {
                return Err(format!("QueryDisplayConfig failed: {}", result.0));
            }
        }

        Err("QueryDisplayConfig kept returning insufficient buffer".to_string())
    }

    fn target_device_name(
        path: &DISPLAYCONFIG_PATH_INFO,
    ) -> Option<DISPLAYCONFIG_TARGET_DEVICE_NAME> {
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
        target.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            size: mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
            adapterId: path.targetInfo.adapterId,
            id: path.targetInfo.id,
        };

        let result = unsafe { DisplayConfigGetDeviceInfo(&mut target.header as *mut _ as *mut _) };
        if result == 0 {
            Some(target)
        } else {
            None
        }
    }

    fn source_device_name(path: &DISPLAYCONFIG_PATH_INFO) -> Option<String> {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        source.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            size: mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
            adapterId: path.sourceInfo.adapterId,
            id: path.sourceInfo.id,
        };

        let result = unsafe { DisplayConfigGetDeviceInfo(&mut source.header as *mut _ as *mut _) };
        if result == 0 {
            let name = wide_to_string(&source.viewGdiDeviceName);
            if name.is_empty() {
                None
            } else {
                Some(name)
            }
        } else {
            None
        }
    }

    fn source_mode_lookup(
        modes: &[DISPLAYCONFIG_MODE_INFO],
    ) -> HashMap<((i32, u32), u32), (u32, u32, i32, i32)> {
        let mut lookup = HashMap::new();
        for mode in modes {
            if mode.infoType != DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE {
                continue;
            }
            let source = unsafe { mode.Anonymous.sourceMode };
            lookup.insert(
                (luid_key(mode.adapterId), mode.id),
                (
                    source.width,
                    source.height,
                    source.position.x,
                    source.position.y,
                ),
            );
        }
        lookup
    }

    fn make_mode_snapshot(
        id: &str,
        device_name: &str,
        friendly: &str,
        monitor_path: &str,
        active: bool,
        gdi: Option<&GdiDisplay>,
        source_mode: Option<&(u32, u32, i32, i32)>,
    ) -> Option<DisplayModeSnapshot> {
        if let Some(gdi_mode) = gdi.and_then(|display| display.mode.clone()) {
            let mut mode = gdi_mode;
            mode.id = id.to_string();
            mode.device_string = friendly.to_string();
            mode.hardware_id = monitor_path.to_string();
            mode.active = active;
            return Some(mode);
        }

        if device_name.is_empty() {
            return None;
        }

        let mut mode = query_display_mode(device_name, active)?;
        if let Some((width, height, x, y)) = source_mode {
            if *width > 0 && *height > 0 {
                mode.width = *width;
                mode.height = *height;
                mode.x = *x;
                mode.y = *y;
            }
        }
        mode.id = id.to_string();
        mode.device_string = friendly.to_string();
        mode.hardware_id = monitor_path.to_string();
        mode.active = active;
        Some(mode)
    }

    fn enumerate_gdi_displays() -> Vec<GdiDisplay> {
        let mut index = 0;
        let mut displays = Vec::new();

        loop {
            let mut device = DISPLAY_DEVICEW::default();
            device.cb = mem::size_of::<DISPLAY_DEVICEW>() as u32;

            let ok = unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &mut device, 0) };
            if !ok.as_bool() {
                break;
            }

            let device_name = wide_to_string(&device.DeviceName);
            let adapter_name = wide_to_string(&device.DeviceString);
            let monitor = monitor_child_for_gdi_display(&device_name);
            let active = device.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0;
            let primary = device.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE != 0;
            let mode = query_display_mode(&device_name, active).map(|mut mode| {
                mode.id = device_name.clone();
                mode.device_string = adapter_name.clone();
                mode.hardware_id = monitor
                    .as_ref()
                    .map(|child| child.1.clone())
                    .unwrap_or_else(|| wide_to_string(&device.DeviceID));
                mode.active = active;
                mode.primary = primary;
                mode
            });

            displays.push(GdiDisplay {
                device_name,
                adapter_name,
                monitor_name: monitor
                    .as_ref()
                    .map(|child| child.0.clone())
                    .unwrap_or_default(),
                monitor_id: monitor
                    .as_ref()
                    .map(|child| child.1.clone())
                    .unwrap_or_else(|| wide_to_string(&device.DeviceID)),
                active,
                primary,
                mode,
            });

            index += 1;
        }

        displays
    }

    fn monitor_child_for_gdi_display(device_name: &str) -> Option<(String, String)> {
        let device_name_wide = to_wide(device_name);
        let mut best = None;
        let mut index = 0;

        loop {
            let mut monitor = DISPLAY_DEVICEW::default();
            monitor.cb = mem::size_of::<DISPLAY_DEVICEW>() as u32;
            let ok = unsafe {
                EnumDisplayDevicesW(PCWSTR(device_name_wide.as_ptr()), index, &mut monitor, 0)
            };
            if !ok.as_bool() {
                break;
            }

            let name = wide_to_string(&monitor.DeviceString);
            let id = wide_to_string(&monitor.DeviceID);
            if best.is_none() || monitor.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0 {
                best = Some((name, id));
            }

            index += 1;
        }

        best
    }

    fn find_gdi_for_target(
        monitor_path: &str,
        friendly: &str,
        gdi_displays: &[GdiDisplay],
    ) -> Option<GdiDisplay> {
        let normalized_path = normalize_id(monitor_path);
        let model_key = monitor_model_key(monitor_path);
        let friendly_upper = friendly.to_uppercase();

        gdi_displays
            .iter()
            .find(|display| {
                let monitor_id = normalize_id(&display.monitor_id);
                !monitor_id.is_empty()
                    && (!normalized_path.is_empty()
                        && (normalized_path.contains(&monitor_id)
                            || monitor_id.contains(&normalized_path)))
            })
            .or_else(|| {
                gdi_displays.iter().find(|display| {
                    model_key
                        .as_ref()
                        .zip(monitor_model_key(&display.monitor_id).as_ref())
                        .map(|(target_model, display_model)| target_model == display_model)
                        .unwrap_or(false)
                })
            })
            .or_else(|| {
                gdi_displays.iter().find(|display| {
                    !friendly_upper.is_empty()
                        && !display.monitor_name.trim().is_empty()
                        && (display
                            .monitor_name
                            .to_uppercase()
                            .contains(&friendly_upper)
                            || friendly_upper.contains(&display.monitor_name.to_uppercase()))
                })
            })
            .cloned()
    }

    fn monitor_model_key(value: &str) -> Option<String> {
        let cleaned = value.trim().trim_start_matches("\\\\?\\");
        let parts = cleaned
            .split(['#', '\\'])
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>();

        parts.windows(2).find_map(|window| {
            if window[0].eq_ignore_ascii_case("DISPLAY")
                || window[0].eq_ignore_ascii_case("MONITOR")
            {
                Some(window[1].to_uppercase())
            } else {
                None
            }
        })
    }

    fn query_display_mode(device_name: &str, active: bool) -> Option<DisplayModeSnapshot> {
        let device_name_wide = to_wide(device_name);
        let attempts = if active {
            vec![
                (ENUM_CURRENT_SETTINGS, EDS_RAWMODE),
                (ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS(0)),
                (ENUM_REGISTRY_SETTINGS, EDS_RAWMODE),
                (ENUM_REGISTRY_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS(0)),
            ]
        } else {
            vec![
                (ENUM_REGISTRY_SETTINGS, EDS_RAWMODE),
                (ENUM_REGISTRY_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS(0)),
                (ENUM_CURRENT_SETTINGS, EDS_RAWMODE),
                (ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS(0)),
            ]
        };

        let mut devmode = None;
        for (mode, flags) in attempts {
            if let Some(candidate) = read_display_mode(&device_name_wide, mode, flags) {
                devmode = Some(candidate);
                break;
            }
        }

        if devmode.is_none() {
            devmode = first_supported_display_mode(&device_name_wide);
        }

        let devmode = devmode?;
        let display = unsafe { devmode.Anonymous1.Anonymous2 };
        Some(DisplayModeSnapshot {
            id: device_name.to_string(),
            device_name: device_name.to_string(),
            device_string: String::new(),
            hardware_id: String::new(),
            active,
            primary: false,
            x: display.dmPosition.x,
            y: display.dmPosition.y,
            width: devmode.dmPelsWidth,
            height: devmode.dmPelsHeight,
            bits_per_pel: devmode.dmBitsPerPel,
            frequency: devmode.dmDisplayFrequency,
            orientation: display.dmDisplayOrientation.0,
            fixed_output: display.dmDisplayFixedOutput.0,
        })
    }

    fn read_display_mode(
        device_name_wide: &[u16],
        mode: ENUM_DISPLAY_SETTINGS_MODE,
        flags: ENUM_DISPLAY_SETTINGS_FLAGS,
    ) -> Option<DEVMODEW> {
        let mut devmode = DEVMODEW::default();
        devmode.dmSize = mem::size_of::<DEVMODEW>() as u16;
        let ok = unsafe {
            EnumDisplaySettingsExW(PCWSTR(device_name_wide.as_ptr()), mode, &mut devmode, flags)
        };
        if ok.as_bool() && devmode.dmPelsWidth > 0 && devmode.dmPelsHeight > 0 {
            Some(devmode)
        } else {
            None
        }
    }

    fn first_supported_display_mode(device_name_wide: &[u16]) -> Option<DEVMODEW> {
        for index in 0..128 {
            for flags in [EDS_RAWMODE, ENUM_DISPLAY_SETTINGS_FLAGS(0)] {
                if let Some(mode) =
                    read_display_mode(device_name_wide, ENUM_DISPLAY_SETTINGS_MODE(index), flags)
                {
                    return Some(mode);
                }
            }
        }

        None
    }

    fn apply_mode(mode: &DisplayModeSnapshot, defer: bool) -> Result<(), String> {
        if mode.device_name.is_empty() {
            return Err(format!("display has no GDI device name: {}", mode.id));
        }
        if mode.width == 0 || mode.height == 0 {
            return Err(format!("display mode is invalid: {}", mode.device_name));
        }

        let mut devmode = DEVMODEW::default();
        devmode.dmSize = mem::size_of::<DEVMODEW>() as u16;
        devmode.dmFields = DM_POSITION
            | DM_PELSWIDTH
            | DM_PELSHEIGHT
            | DM_DISPLAYORIENTATION
            | DM_DISPLAYFIXEDOUTPUT;
        devmode.dmPelsWidth = mode.width;
        devmode.dmPelsHeight = mode.height;
        devmode.dmBitsPerPel = mode.bits_per_pel.max(32);
        devmode.dmDisplayFrequency = mode.frequency;
        if devmode.dmBitsPerPel > 0 {
            devmode.dmFields |= DM_BITSPERPEL;
        }
        if devmode.dmDisplayFrequency > 0 {
            devmode.dmFields |= DM_DISPLAYFREQUENCY;
        }
        devmode.Anonymous1.Anonymous2 = DEVMODEW_0_1 {
            dmPosition: POINTL {
                x: mode.x,
                y: mode.y,
            },
            dmDisplayOrientation: DEVMODE_DISPLAY_ORIENTATION(mode.orientation),
            dmDisplayFixedOutput: DEVMODE_DISPLAY_FIXED_OUTPUT(mode.fixed_output),
        };

        change_display_settings(&mode.device_name, &devmode, defer, "set display mode")
    }

    fn detach_display(device_name: &str, friendly_name: &str) -> Result<(), String> {
        let mut devmode = DEVMODEW::default();
        devmode.dmSize = mem::size_of::<DEVMODEW>() as u16;
        devmode.dmFields = DM_POSITION | DM_PELSWIDTH | DM_PELSHEIGHT;
        devmode.dmPelsWidth = 0;
        devmode.dmPelsHeight = 0;
        devmode.Anonymous1.Anonymous2 = DEVMODEW_0_1 {
            dmPosition: POINTL { x: 0, y: 0 },
            dmDisplayOrientation: DEVMODE_DISPLAY_ORIENTATION(0),
            dmDisplayFixedOutput: DEVMODE_DISPLAY_FIXED_OUTPUT(0),
        };

        change_display_settings(
            device_name,
            &devmode,
            true,
            &format!("detach display {friendly_name}"),
        )
    }

    fn change_display_settings(
        device_name: &str,
        devmode: &DEVMODEW,
        defer: bool,
        action: &str,
    ) -> Result<(), String> {
        let device_name_wide = to_wide(device_name);
        let flags = if defer {
            CDS_UPDATEREGISTRY | CDS_NORESET
        } else {
            CDS_UPDATEREGISTRY
        };
        let result = unsafe {
            ChangeDisplaySettingsExW(
                PCWSTR(device_name_wide.as_ptr()),
                Some(devmode as *const _),
                None,
                flags,
                None,
            )
        };

        if result == DISP_CHANGE_SUCCESSFUL {
            Ok(())
        } else {
            Err(format!("{action} failed, Win32 code {}", result.0))
        }
    }

    fn apply_pending_changes() -> Result<(), String> {
        let result =
            unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE(0), None) };

        if result == DISP_CHANGE_SUCCESSFUL {
            Ok(())
        } else {
            Err(format!(
                "apply display config failed, Win32 code {}",
                result.0
            ))
        }
    }

    fn monitor_display_name(
        friendly: &str,
        monitor_path: &str,
        gdi: Option<&GdiDisplay>,
        rank: usize,
    ) -> String {
        if !friendly.trim().is_empty() {
            return friendly.to_string();
        }
        if !monitor_path.trim().is_empty() {
            return compact_monitor_path(monitor_path);
        }
        if let Some(display) = gdi {
            if !display.monitor_name.trim().is_empty() {
                return display.monitor_name.clone();
            }
            if !display.adapter_name.trim().is_empty() {
                return display.adapter_name.clone();
            }
        }
        format!("Display {}", rank + 1)
    }

    fn is_virtual_display(name: &str, path: &str, device: &str) -> bool {
        let combined = format!("{name} {path} {device}").to_uppercase();
        combined.contains("VIRTUAL") || combined.contains("VDD") || combined.contains("MTT")
    }

    fn compact_monitor_path(path: &str) -> String {
        let cleaned = path.trim_start_matches("\\\\?\\");
        let parts = cleaned
            .split('#')
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>();

        if parts
            .first()
            .map(|part| {
                part.eq_ignore_ascii_case("DISPLAY") || part.eq_ignore_ascii_case("MONITOR")
            })
            .unwrap_or(false)
        {
            if let Some(model) = parts.get(1) {
                return (*model).to_string();
            }
        }

        parts
            .iter()
            .find(|part| {
                let upper = part.to_uppercase();
                upper.contains("DISPLAY") || upper.contains("MONITOR")
            })
            .copied()
            .unwrap_or(cleaned)
            .to_string()
    }

    fn normalize_id(value: &str) -> String {
        value
            .trim()
            .trim_start_matches(r"\\?\")
            .replace('\\', "#")
            .to_uppercase()
    }

    fn luid_key(luid: LUID) -> (i32, u32) {
        (luid.HighPart, luid.LowPart)
    }

    fn wide_to_string(value: &[u16]) -> String {
        let len = value.iter().position(|c| *c == 0).unwrap_or(value.len());
        String::from_utf16_lossy(&value[..len]).trim().to_string()
    }

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(windows))]
mod native {
    use super::{DisplayLayoutSnapshot, MonitorInfo};

    pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>, String> {
        Err("display management is only supported on Windows".to_string())
    }

    pub fn capture_layout() -> Result<DisplayLayoutSnapshot, String> {
        Err("display management is only supported on Windows".to_string())
    }

    pub fn switch_to_display(_display_id: &str) -> Result<(), String> {
        Err("display management is only supported on Windows".to_string())
    }

    pub fn restore_layout(_snapshot: &DisplayLayoutSnapshot) -> Result<(), String> {
        Err("display management is only supported on Windows".to_string())
    }
}

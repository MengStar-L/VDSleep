use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use sysinfo::System;

#[cfg(windows)]
use std::net::Ipv4Addr;

pub struct VdMonitor {
    running: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    vd_running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
}

impl VdMonitor {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            connected: Arc::new(AtomicBool::new(false)),
            vd_running: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
    }

    pub fn is_vd_running(&self) -> bool {
        self.vd_running.load(Ordering::Relaxed)
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn refresh_now(&self) -> bool {
        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        let vd_is_running = sys.processes().values().any(|process| {
            let name = process.name().to_string_lossy();
            name.contains("VirtualDesktop.Streamer") || name.contains("Virtual Desktop Streamer")
        });

        self.vd_running.store(vd_is_running, Ordering::Relaxed);

        let connected = if vd_is_running {
            check_vd_connections(&sys)
        } else {
            false
        };

        self.connected.store(connected, Ordering::Relaxed);
        connected
    }

    pub fn start<F>(&self, on_state_change: F)
    where
        F: Fn(bool) + Send + 'static,
    {
        self.running.store(true, Ordering::Relaxed);
        let running = self.running.clone();
        let connected = self.connected.clone();
        let vd_running = self.vd_running.clone();
        let paused = self.paused.clone();

        thread::spawn(move || {
            let mut sys = System::new();
            let mut last_connected = connected.load(Ordering::Relaxed);
            let mut debounce_count: u32 = 0;
            let mut pending_state: Option<bool> = None;
            let debounce_threshold: u32 = 3;

            while running.load(Ordering::Relaxed) {
                if paused.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }

                sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

                let vd_is_running = sys.processes().values().any(|process| {
                    let name = process.name().to_string_lossy();
                    name.contains("VirtualDesktop.Streamer")
                        || name.contains("Virtual Desktop Streamer")
                });
                vd_running.store(vd_is_running, Ordering::Relaxed);

                let raw_connected = if vd_is_running {
                    check_vd_connections(&sys)
                } else {
                    false
                };

                if !vd_is_running {
                    last_connected = false;
                    pending_state = None;
                    debounce_count = 0;

                    if connected.load(Ordering::Relaxed) {
                        connected.store(false, Ordering::Relaxed);
                        on_state_change(false);
                    }
                } else if raw_connected != last_connected {
                    match pending_state {
                        Some(state) if state == raw_connected => {
                            debounce_count += 1;
                            let current_threshold =
                                if raw_connected { 1 } else { debounce_threshold };
                            if debounce_count >= current_threshold {
                                last_connected = raw_connected;
                                connected.store(raw_connected, Ordering::Relaxed);
                                on_state_change(raw_connected);
                                pending_state = None;
                                debounce_count = 0;
                            }
                        }
                        _ => {
                            pending_state = Some(raw_connected);
                            debounce_count = 1;
                        }
                    }
                } else {
                    pending_state = None;
                    debounce_count = 0;
                }

                thread::sleep(Duration::from_secs(1));
            }
        });
    }
}

fn check_vd_connections(sys: &System) -> bool {
    #[cfg(windows)]
    {
        for process in sys.processes().values() {
            let name = process.name().to_string_lossy();
            if name.contains("VirtualDesktop.Streamer") || name.contains("Virtual Desktop Streamer")
            {
                if let Ok(connections) = get_tcp_connections_for_pid(process.pid().as_u32()) {
                    let mut lan_ip_counts: std::collections::HashMap<Ipv4Addr, u32> =
                        std::collections::HashMap::new();
                    for (remote_ip, _) in &connections {
                        if is_lan_ip(*remote_ip) {
                            *lan_ip_counts.entry(*remote_ip).or_insert(0) += 1;
                        }
                    }
                    for count in lan_ip_counts.values() {
                        if *count >= 3 {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        let _ = sys;
        false
    }
}

#[cfg(windows)]
fn is_lan_ip(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 192 && octets[1] == 168
        || octets[0] == 10
        || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
}

#[cfg(windows)]
fn get_tcp_connections_for_pid(
    target_pid: u32,
) -> Result<Vec<(Ipv4Addr, u16)>, Box<dyn std::error::Error>> {
    use std::mem;
    use windows::Win32::NetworkManagement::IpHelper::*;
    use windows::Win32::Networking::WinSock::*;

    let mut connections = Vec::new();

    unsafe {
        let mut size: u32 = 0;
        let _ = GetExtendedTcpTable(
            None,
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_CLASS(5),
            0,
        );

        if size == 0 {
            return Ok(connections);
        }

        let mut buffer: Vec<u8> = vec![0u8; size as usize];
        let result = GetExtendedTcpTable(
            Some(buffer.as_mut_ptr() as *mut _),
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_CLASS(5),
            0,
        );

        if result != 0 {
            return Ok(connections);
        }

        let num_entries = *(buffer.as_ptr() as *const u32);
        let entries_ptr = buffer.as_ptr().add(mem::size_of::<u32>()) as *const MIB_TCPROW_OWNER_PID;

        for i in 0..num_entries as usize {
            let entry = &*entries_ptr.add(i);
            if entry.dwOwningPid == target_pid && entry.dwState == 5 {
                let remote_ip = Ipv4Addr::from(entry.dwRemoteAddr.to_ne_bytes());
                let remote_port = u16::from_be(entry.dwRemotePort as u16);
                if remote_ip != Ipv4Addr::UNSPECIFIED {
                    connections.push((remote_ip, remote_port));
                }
            }
        }
    }

    Ok(connections)
}

#[cfg(windows)]
#[repr(C)]
#[allow(non_snake_case, non_camel_case_types, dead_code)]
struct MIB_TCPROW_OWNER_PID {
    dwState: u32,
    dwLocalAddr: u32,
    dwLocalPort: u32,
    dwRemoteAddr: u32,
    dwRemotePort: u32,
    dwOwningPid: u32,
}

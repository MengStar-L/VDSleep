#[cfg(windows)]
use std::collections::HashSet;

pub struct AudioController;

impl AudioController {
    pub fn new() -> Self {
        Self
    }

    pub fn get_mute_state(&self) -> bool {
        #[cfg(windows)]
        {
            match get_mute_native() {
                Ok(muted) => muted,
                Err(error) => {
                    eprintln!("[Audio] 获取静音状态失败: {error}");
                    false
                }
            }
        }
        #[cfg(not(windows))]
        false
    }

    pub fn set_mute(&self, mute: bool) -> bool {
        #[cfg(windows)]
        {
            match set_mute_native(mute) {
                Ok(_) => true,
                Err(error) => {
                    eprintln!("[Audio] 设置静音失败: {error}");
                    false
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = mute;
            false
        }
    }

    pub fn toggle_mute(&self) -> (bool, bool) {
        let muted = !self.get_mute_state();
        (self.set_mute(muted), muted)
    }

    pub fn zero_unprotected_render_devices(
        &self,
        protected_ids: &mut HashSet<String>,
    ) -> (bool, usize) {
        #[cfg(windows)]
        {
            match zero_unprotected_render_devices_native(protected_ids) {
                Ok(changed) => (true, changed),
                Err(error) => {
                    eprintln!("[Audio] 设置播放设备音量为 0 失败: {error}");
                    (false, 0)
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = protected_ids;
            (false, 0)
        }
    }
}

#[cfg(windows)]
fn get_endpoint_volume(
) -> Result<windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume, String> {
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|error| format!("创建设备枚举器失败: {error}"))?;

        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|error| format!("获取默认音频设备失败: {error}"))?;

        device
            .Activate(CLSCTX_ALL, None)
            .map_err(|error| format!("激活音量接口失败: {error}"))
    }
}

#[cfg(windows)]
fn get_mute_native() -> Result<bool, String> {
    unsafe {
        let volume = get_endpoint_volume()?;
        let muted = volume
            .GetMute()
            .map_err(|error| format!("GetMute 失败: {error}"))?;
        Ok(muted.as_bool())
    }
}

#[cfg(windows)]
fn set_mute_native(mute: bool) -> Result<(), String> {
    use windows::Win32::Foundation::BOOL;

    unsafe {
        let volume = get_endpoint_volume()?;
        volume
            .SetMute(BOOL::from(mute), std::ptr::null())
            .map_err(|error| format!("SetMute 失败: {error}"))?;
        Ok(())
    }
}

#[cfg(windows)]
fn zero_unprotected_render_devices_native(
    protected_ids: &mut HashSet<String>,
) -> Result<usize, String> {
    let devices = get_render_endpoint_volumes()?;
    let mut changed = 0usize;

    for (id, volume) in devices {
        if protected_ids.contains(&id) {
            continue;
        }

        unsafe {
            volume
                .SetMasterVolumeLevelScalar(0.0, std::ptr::null())
                .map_err(|error| format!("SetMasterVolumeLevelScalar 失败: {error}"))?;
        }

        protected_ids.insert(id);
        changed += 1;
    }

    Ok(changed)
}

#[cfg(windows)]
fn get_render_endpoint_volumes() -> Result<
    Vec<(
        String,
        windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume,
    )>,
    String,
> {
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|error| format!("创建音频设备枚举器失败: {error}"))?;

        let collection = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .map_err(|error| format!("枚举播放设备失败: {error}"))?;

        let count = collection
            .GetCount()
            .map_err(|error| format!("读取播放设备数量失败: {error}"))?;

        let mut devices = Vec::with_capacity(count as usize);
        for index in 0..count {
            let device = collection
                .Item(index)
                .map_err(|error| format!("读取播放设备失败: {error}"))?;
            let id = get_device_id(&device)?;
            let volume = activate_endpoint_volume(&device)?;
            devices.push((id, volume));
        }

        Ok(devices)
    }
}

#[cfg(windows)]
fn activate_endpoint_volume(
    device: &windows::Win32::Media::Audio::IMMDevice,
) -> Result<windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume, String> {
    use windows::Win32::System::Com::CLSCTX_ALL;

    unsafe {
        device
            .Activate(CLSCTX_ALL, None)
            .map_err(|error| format!("激活音量接口失败: {error}"))
    }
}

#[cfg(windows)]
fn get_device_id(device: &windows::Win32::Media::Audio::IMMDevice) -> Result<String, String> {
    use windows::core::PWSTR;
    use windows::Win32::System::Com::CoTaskMemFree;

    unsafe {
        let raw_id: PWSTR = device
            .GetId()
            .map_err(|error| format!("读取设备 ID 失败: {error}"))?;
        let id = raw_id
            .to_string()
            .map_err(|error| format!("解析设备 ID 失败: {error}"))?;
        CoTaskMemFree(Some(raw_id.0 as _));
        Ok(id)
    }
}

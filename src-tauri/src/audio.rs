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

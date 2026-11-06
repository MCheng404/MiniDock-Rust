//! 系统音量：Core Audio 的 `IAudioEndpointVolume`。
//!
//! **不用模拟 `VK_VOLUME_*` 按键**：那会改动系统
//! 的"应用程序音量"分组、会在音量 OSD 上闪一下，而且拿不到真实数值。
//!
//! 三件事要注意：
//!
//! - **COM 必须在调用线程初始化**。这个结构体只在 UI 线程用，所以由
//!   [`Audio::new`] 顺手 `CoInitializeEx`。已初始化过（比如宿主程序先做过）
//!   会返回 `RPC_E_CHANGED_MODE`，那不是错误，忽略即可。
//! - **接口指针不能跨线程**（默认是 apartment 模型），所以 `Audio` 不是
//!   `Send`/`Sync` —— 编译器会替我们守住这条。
//! - **失败要能降级**：没有音频设备的机器（服务器、禁用了音频服务）上
//!   构造会失败，调用方应当把滑块画成禁用态而不是崩掉。

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};

/// 默认播放设备的音量控制。
pub struct Audio {
    epv: IAudioEndpointVolume,
}

impl std::fmt::Debug for Audio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audio").finish_non_exhaustive()
    }
}

impl Audio {
    /// 打开默认播放设备。没有设备 / 音频服务没起来时返回 `None`。
    #[must_use]
    pub fn new() -> Option<Self> {
        unsafe {
            // 已经初始化过会返回 RPC_E_CHANGED_MODE（多线程 apartment），
            // 那不影响我们用默认设备的接口，忽略。
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
            let epv: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
            Some(Self { epv })
        }
    }

    /// 当前主音量，0..=100。取不到时返回 `None`。
    #[must_use]
    pub fn volume(&self) -> Option<u32> {
        let v = unsafe { self.epv.GetMasterVolumeLevelScalar() }.ok()?;
        Some((v * 100.0 + 0.5).clamp(0.0, 100.0) as u32)
    }

    /// 设主音量。`v` 会被夹到 0..=100。
    pub fn set_volume(&self, v: u32) {
        let v = v.min(100) as f32 / 100.0;
        unsafe {
            let _ = self.epv.SetMasterVolumeLevelScalar(v, std::ptr::null());
        }
    }

    /// 当前是否静音。
    #[must_use]
    pub fn muted(&self) -> bool {
        unsafe { self.epv.GetMute() }
            .map(|m| m.as_bool())
            .unwrap_or(false)
    }

    /// 设置静音。
    pub fn set_muted(&self, muted: bool) {
        unsafe {
            let _ = self.epv.SetMute(muted, std::ptr::null());
        }
    }

    /// 反过来。
    pub fn toggle_mute(&self) -> bool {
        let next = !self.muted();
        self.set_muted(next);
        next
    }
}

/// 电池状态（控制中心底部那行字）。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct Battery {
    /// 电量百分比；`None` 表示没有电池（台式机）。
    pub percent: Option<u8>,
    /// 是否接着交流电。
    pub ac: bool,
}

impl Battery {
    /// 读一次。用的是 `GetSystemPowerStatus`。
    #[must_use]
    pub fn read() -> Self {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut st = SYSTEM_POWER_STATUS::default();
        if unsafe { GetSystemPowerStatus(&mut st) }.is_err() {
            return Self::default();
        }
        Self {
            // 255 = 未知（台式机没有电池时就是这个值）。
            percent: (st.BatteryLifePercent != 255).then_some(st.BatteryLifePercent),
            ac: st.ACLineStatus == 1,
        }
    }

    /// 底部那行文案。
    #[must_use]
    pub fn text(self) -> String {
        match self.percent {
            Some(p) if self.ac => format!("电量 {p}% · 已接通电源"),
            Some(p) => format!("电量 {p}%"),
            None => "未检测到电池".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 电池文案：三种情况都要有话说，不能空白。
    #[test]
    fn battery_text_covers_all_cases() {
        assert_eq!(
            Battery {
                percent: Some(80),
                ac: true
            }
            .text(),
            "电量 80% · 已接通电源"
        );
        assert_eq!(
            Battery {
                percent: Some(35),
                ac: false
            }
            .text(),
            "电量 35%"
        );
        assert_eq!(Battery::default().text(), "未检测到电池");
    }
}

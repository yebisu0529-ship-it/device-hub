mod onboard;

use crate::transport::Channel;
use devhub_core::backend::*;
use devhub_core::model::*;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const SLOT: u8 = 1;
const REPORT: u8 = 0x11;
const POLLING: &str = "polling_wireless";
const ONBOARD_ENABLE: &str = "onboard_enable";
const ONBOARD_DISABLE: &str = "onboard_disable";

struct Logitech {
    channel: Mutex<Channel>,
    info: DeviceInfo,
    caps: Capabilities,
    dpi: u8,
    polling: u8,
    onboard: u8,
}

fn request(channel: &Channel, feature: u8, function: u8, params: &[u8]) -> HubResult<Vec<u8>> {
    let header = [SLOT, feature, (function << 4) | 0x0d];
    let mut body = [0; 19];
    body[..3].copy_from_slice(&header);
    body[3..3 + params.len()].copy_from_slice(params);
    channel.send_report(REPORT, &body)?;
    let deadline = Instant::now() + Duration::from_millis(500);
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        let reply = channel.recv(remaining.as_millis().max(1) as i32)?;
        if !matches!(reply.first(), Some(0x10 | 0x11)) {
            continue;
        }
        if reply.get(1..4) == Some(header.as_slice()) && reply.len() >= 7 {
            return Ok(reply[4..].to_vec());
        }
        if reply.get(1..5) == Some(&[SLOT, 0xff, feature, header[2]]) && reply.len() >= 6 {
            return Err(HubError::Protocol(format!(
                "HID++ error 0x{:02x}",
                reply[5]
            )));
        }
    }
    Err(HubError::Protocol("HID++ reply timeout".into()))
}

fn feature(channel: &Channel, id: u16) -> HubResult<u8> {
    match request(channel, 0, 0, &id.to_be_bytes())?[0] {
        0 => Err(HubError::Unsupported("HID++ feature")),
        index => Ok(index),
    }
}

pub fn open(device: hidapi::HidDevice, info: DeviceInfo) -> HubResult<impl DeviceDriver> {
    let channel = Channel::open(device, 20, 20);
    let name_feature = feature(&channel, 0x0005)?;
    let name = request(&channel, name_feature, 1, &[0])?;
    if !name.starts_with(b"G Pro Wireless ") {
        return Err(HubError::Unsupported("POWERPLAY paired mouse"));
    }
    let dpi = feature(&channel, 0x2201)?;
    let polling = feature(&channel, 0x8060)?;
    let onboard = feature(&channel, 0x8100)?;
    onboard::validate(&channel, onboard)?;
    let mask = request(&channel, polling, 0, &[])?[0];
    let rates = [1, 2, 4, 8]
        .into_iter()
        .filter(|&ms| mask & (1 << (ms - 1)) != 0)
        .map(|ms| ChoiceOpt::new(1000 / ms, &format!("{} Hz", 1000 / ms)))
        .collect();

    let mut caps = Capabilities::empty();
    caps.settings.push(SettingSpec::choice(
        POLLING,
        section::PERFORMANCE,
        "ポーリングレート",
        rates,
    ));
    caps.actions.extend([
        ActionSpec::new(
            ONBOARD_ENABLE,
            section::PERFORMANCE,
            "オンボードモードをオン",
        )
        .confirm("マウスに保存されたボタン割り当て、DPI、ポーリングレートが有効になります。"),
        ActionSpec::new(
            ONBOARD_DISABLE,
            section::PERFORMANCE,
            "オンボードモードをオフ",
        )
        .help("G HUB などのアプリで設定するモードに戻します。"),
    ]);
    caps.mouse = Some(MouseCaps {
        dpi_min: 100,
        dpi_max: 25600,
        dpi_step: 50,
        commit_dpi_on_release: true,
        min_stages: 1,
        max_stages: 1,
        xy_split: false,
        stage_colors: false,
        stage_toggle: false,
        buttons: onboard::LABELS
            .into_iter()
            .map(|name| ButtonSlot { name: name.into() })
            .collect(),
        button_kinds: vec![
            ButtonActionKind::Mouse,
            ButtonActionKind::Key,
            ButtonActionKind::Media,
            ButtonActionKind::Dpi,
            ButtonActionKind::ProfileCycle,
            ButtonActionKind::Disabled,
        ],
        button_options: Some(onboard::button_options()),
        require_left_click: true,
        macros: None,
        live_buttons: vec![],
        adaptive_dpi: None,
    });
    Ok(Logitech {
        channel: Mutex::new(channel),
        info,
        caps,
        dpi,
        polling,
        onboard,
    })
}

impl Logitech {
    fn save_profile(&self, channel: &Channel, profile: onboard::Profile) -> HubResult<()> {
        let active = onboard::mode(channel, self.onboard)?;
        let previous_dpi = if active {
            Some(request(channel, self.dpi, 2, &[0])?)
        } else {
            None
        };
        profile.write(channel)?;
        if let Some(old) = previous_dpi {
            if request(channel, self.dpi, 2, &[0])?[1..3] != old[1..3] {
                request(channel, self.dpi, 3, &old[..3])?;
                if request(channel, self.dpi, 2, &[0])?[1..3] != old[1..3] {
                    return Err(HubError::Protocol(
                        "HID++ active DPI restoration failed".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

impl DeviceDriver for Logitech {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn caps(&self) -> Capabilities {
        self.caps.clone()
    }

    fn read(&self) -> HubResult<DeviceState> {
        let channel = self.channel.lock().unwrap_or_else(|err| err.into_inner());
        let dpi = request(&channel, self.dpi, 2, &[0])?;
        let ms = request(&channel, self.polling, 1, &[])?[0];
        if ms == 0 {
            return Err(HubError::Protocol("HID++ report interval is zero".into()));
        }
        let profile = onboard::Profile::read(&channel, self.onboard)?;
        Ok(DeviceState {
            profile: ProfileState {
                active: 0,
                names: vec![],
            },
            settings: [(POLLING.into(), SettingValue::Choice(1000 / i64::from(ms)))].into(),
            mouse: Some(MouseState {
                stages: vec![DpiStage {
                    x: u32::from(u16::from_be_bytes([dpi[1], dpi[2]])),
                    y: None,
                    color: Rgb::hex(0xffffff),
                    enabled: true,
                }],
                active_stage: 0,
                buttons: profile.buttons(),
                macros: vec![],
                adaptive: None,
            }),
            keyboard: None,
            lighting: Default::default(),
            peq: None,
        })
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        let channel = self.channel.lock().unwrap_or_else(|err| err.into_inner());
        match change {
            Change::Setting {
                id,
                value: SettingValue::Choice(hz),
            } if id == POLLING => {
                let ms = match hz {
                    1000 => 1,
                    500 => 2,
                    250 => 4,
                    125 => 8,
                    _ => return Err(HubError::Invalid("HID++ polling rate".into())),
                };
                let mask = request(&channel, self.polling, 0, &[])?[0];
                if mask & (1 << (ms - 1)) == 0 {
                    return Err(HubError::Unsupported("HID++ polling rate"));
                }
                if onboard::mode(&channel, self.onboard)? {
                    let mut profile = onboard::Profile::read(&channel, self.onboard)?;
                    if profile.set_interval(ms) {
                        self.save_profile(&channel, profile)?;
                    }
                } else {
                    request(&channel, self.polling, 2, &[ms])?;
                }
                if request(&channel, self.polling, 1, &[])?[0] != ms {
                    return Err(HubError::Protocol(
                        "HID++ polling rate did not change".into(),
                    ));
                }
            }
            Change::DpiStages { stages, active: 0 } if stages.len() == 1 => {
                let stage = stages[0];
                if !(100..=25600).contains(&stage.x)
                    || stage.x % 50 != 0
                    || stage.y.is_some()
                    || !stage.enabled
                {
                    return Err(HubError::Invalid("HID++ DPI".into()));
                }
                let dpi = stage.x as u16;
                if onboard::mode(&channel, self.onboard)? {
                    let index = request(&channel, self.onboard, 11, &[])?[0];
                    let mut profile = onboard::Profile::read(&channel, self.onboard)?;
                    if profile.set_dpi(index, dpi)? {
                        profile.write(&channel)?;
                    }
                }
                let [hi, lo] = dpi.to_be_bytes();
                if request(&channel, self.dpi, 2, &[0])?[1..3] != [hi, lo] {
                    request(&channel, self.dpi, 3, &[0, hi, lo])?;
                }
                if request(&channel, self.dpi, 2, &[0])?[1..3] != [hi, lo] {
                    return Err(HubError::Protocol("HID++ DPI did not change".into()));
                }
            }
            Change::Button { slot, action } => {
                let mut profile = onboard::Profile::read(&channel, self.onboard)?;
                if profile.set_button(*slot, action)? {
                    self.save_profile(&channel, profile)?;
                }
            }
            Change::ActiveStage(0) => {}
            _ => return Err(HubError::Unsupported("HID++ setting")),
        }
        Ok(())
    }

    fn run_action(&self, id: &str, _file: Option<&std::path::Path>) -> HubResult<ActionOutcome> {
        let enabled = match id {
            ONBOARD_ENABLE => true,
            ONBOARD_DISABLE => false,
            _ => return Err(HubError::Unsupported("HID++ action")),
        };
        let channel = self.channel.lock().unwrap_or_else(|err| err.into_inner());
        if enabled {
            // Refuse to switch to an unreadable profile or one without a left click.
            let profile = onboard::Profile::read(&channel, self.onboard)?;
            if !profile
                .buttons()
                .contains(&ButtonAction::Mouse(MouseFn::Left))
            {
                return Err(HubError::Invalid(
                    "左クリックの割り当てを1つ残してください".into(),
                ));
            }
        }
        onboard::set_mode(&channel, self.onboard, enabled)?;
        Ok(ActionOutcome {
            message: None,
            reload: true,
        })
    }
}

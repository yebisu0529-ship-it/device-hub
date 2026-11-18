use super::request;
use crate::transport::Channel;
use devhub_core::backend::{HubError, HubResult};
use devhub_core::model::{ButtonAction, DpiFn, KeyStroke, MediaFn, MouseFn};

const SIZE: usize = 255;
const BUTTONS: usize = 32;
const BUTTON_BYTES: usize = 4;
const CHECKSUM: usize = SIZE - 2;
pub(super) const LABELS: [&str; 8] = [
    "左クリック",
    "右クリック",
    "ホイールクリック",
    "左サイド 1",
    "左サイド 2",
    "底面 DPI ボタン",
    "右サイド 1",
    "右サイド 2",
];

const MOUSE: [MouseFn; 5] = [
    MouseFn::Left,
    MouseFn::Right,
    MouseFn::Middle,
    MouseFn::Back,
    MouseFn::Forward,
];
const SPECIAL: [(u8, ButtonAction); 8] = [
    (0x01, ButtonAction::Mouse(MouseFn::ScrollLeft)),
    (0x02, ButtonAction::Mouse(MouseFn::ScrollRight)),
    (0x03, ButtonAction::Dpi(DpiFn::Up)),
    (0x04, ButtonAction::Dpi(DpiFn::Down)),
    (0x05, ButtonAction::Dpi(DpiFn::Cycle)),
    (0x0a, ButtonAction::ProfileCycle),
    (0x10, ButtonAction::Mouse(MouseFn::ScrollDown)),
    (0x11, ButtonAction::Mouse(MouseFn::ScrollUp)),
];
const MEDIA: [(u16, MediaFn); 14] = [
    (0xcd, MediaFn::PlayPause),
    (0xb7, MediaFn::Stop),
    (0xb5, MediaFn::Next),
    (0xb6, MediaFn::Prev),
    (0xe9, MediaFn::VolumeUp),
    (0xea, MediaFn::VolumeDown),
    (0xe2, MediaFn::Mute),
    (0x192, MediaFn::Calculator),
    (0x194, MediaFn::MyComputer),
    (0x196, MediaFn::Browser),
    (0x18a, MediaFn::Mail),
    (0x6f, MediaFn::BrightnessUp),
    (0x70, MediaFn::BrightnessDown),
    (0x227, MediaFn::Refresh),
];

pub(super) fn button_options() -> Vec<ButtonAction> {
    MOUSE
        .into_iter()
        .map(ButtonAction::Mouse)
        .chain(SPECIAL.into_iter().map(|(_, action)| action))
        .chain(
            MEDIA
                .into_iter()
                .map(|(_, media)| ButtonAction::Media(media)),
        )
        .collect()
}

fn decode(raw: [u8; 4]) -> ButtonAction {
    let value = u16::from_be_bytes([raw[2], raw[3]]);
    let known = match raw {
        [0xff, ..] | [0x80, 0, ..] => Some(ButtonAction::Disabled),
        [0x80, 1, ..] if value.is_power_of_two() => MOUSE
            .get(value.trailing_zeros() as usize)
            .copied()
            .map(ButtonAction::Mouse),
        [0x80, 2, mods, usage] => Some(ButtonAction::Key(KeyStroke {
            mods,
            usage: usage.into(),
        })),
        [0x80, 3, ..] => MEDIA
            .iter()
            .find(|(code, _)| *code == value)
            .map(|(_, media)| ButtonAction::Media(*media)),
        [0x90, code, ..] => SPECIAL
            .iter()
            .find(|(value, _)| *value == code)
            .map(|(_, action)| action.clone()),
        _ => None,
    };
    known.unwrap_or_else(|| ButtonAction::Unsupported {
        ty: raw[0],
        params: raw[1..].to_vec(),
    })
}

fn encode(action: &ButtonAction) -> HubResult<[u8; 4]> {
    match action {
        ButtonAction::Disabled => return Ok([0xff; 4]),
        ButtonAction::Key(key) => {
            let usage =
                u8::try_from(key.usage).map_err(|_| HubError::Invalid("HID++ key usage".into()))?;
            return Ok([0x80, 2, key.mods, usage]);
        }
        ButtonAction::Mouse(mouse) => {
            if let Some(index) = MOUSE.iter().position(|candidate| candidate == mouse) {
                return Ok([0x80, 1, 0, 1 << index]);
            }
        }
        ButtonAction::Media(media) => {
            if let Some((code, _)) = MEDIA.iter().find(|(_, candidate)| candidate == media) {
                let [hi, lo] = code.to_be_bytes();
                return Ok([0x80, 3, hi, lo]);
            }
        }
        _ => {}
    }
    SPECIAL
        .iter()
        .find(|(_, candidate)| candidate == action)
        .map(|(code, _)| [0x90, *code, 0, 0])
        .ok_or(HubError::Unsupported("HID++ button action"))
}

fn crc(data: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = (crc << 1) ^ if crc & 0x8000 != 0 { 0x1021 } else { 0 };
        }
    }
    crc
}

pub(super) struct Profile {
    feature: u8,
    sector: u16,
    bytes: [u8; SIZE],
}

pub(super) fn validate(channel: &Channel, feature: u8) -> HubResult<()> {
    let info = request(channel, feature, 0, &[])?;
    if !matches!(info.as_slice(), [1, 3, 1, 5, 1, 8, 16, 0, 255, ..]) {
        return Err(HubError::Unsupported("PRO Wireless onboard layout"));
    }
    Ok(())
}

pub(super) fn mode(channel: &Channel, feature: u8) -> HubResult<bool> {
    match request(channel, feature, 2, &[])?[0] {
        1 => Ok(true),
        2 => Ok(false),
        _ => Err(HubError::Protocol("HID++ onboard mode".into())),
    }
}

pub(super) fn set_mode(channel: &Channel, feature: u8, enabled: bool) -> HubResult<()> {
    if mode(channel, feature)? != enabled {
        request(channel, feature, 1, &[if enabled { 1 } else { 2 }])?;
        if mode(channel, feature)? != enabled {
            return Err(HubError::Protocol(
                "HID++ onboard mode did not change".into(),
            ));
        }
    }
    Ok(())
}

fn read_sector(channel: &Channel, feature: u8, sector: u16) -> HubResult<[u8; SIZE]> {
    let mut bytes = [0; SIZE];
    let [sh, sl] = sector.to_be_bytes();
    for pos in (0..SIZE).step_by(16) {
        let start = pos.min(SIZE - 16);
        let [oh, ol] = (start as u16).to_be_bytes();
        let reply = request(channel, feature, 5, &[sh, sl, oh, ol])?;
        let chunk = reply
            .get(..16)
            .ok_or_else(|| HubError::Protocol("Short HID++ memory report".into()))?;
        bytes[start..start + 16].copy_from_slice(chunk);
    }
    if crc(&bytes[..CHECKSUM]).to_be_bytes() != bytes[CHECKSUM..] {
        return Err(HubError::Protocol(format!(
            "HID++ profile sector {sector} checksum mismatch"
        )));
    }
    Ok(bytes)
}

impl Profile {
    pub(super) fn read(channel: &Channel, feature: u8) -> HubResult<Self> {
        let directory = read_sector(channel, feature, 0)?;
        let active = request(channel, feature, 4, &[])?;
        let active = u16::from_be_bytes([active[0], active[1]]);
        let mut enabled = directory
            .chunks_exact(4)
            .map(|entry| (u16::from_be_bytes([entry[0], entry[1]]), entry[2]))
            .take_while(|(sector, _)| *sector != 0xffff)
            .filter(|(sector, enabled)| (1..=5).contains(sector) && *enabled == 1)
            .map(|(sector, _)| sector);
        let sector = if active == 0 {
            enabled.next()
        } else {
            enabled.find(|sector| *sector == active)
        }
        .ok_or(HubError::Unsupported(
            "Writable active PRO Wireless onboard profile",
        ))?;
        let bytes = read_sector(channel, feature, sector)?;
        Ok(Self {
            feature,
            sector,
            bytes,
        })
    }

    pub(super) fn buttons(&self) -> Vec<ButtonAction> {
        self.bytes[BUTTONS..BUTTONS + LABELS.len() * BUTTON_BYTES]
            .chunks_exact(BUTTON_BYTES)
            .map(|binding| decode(binding.try_into().expect("four-byte binding")))
            .collect()
    }

    pub(super) fn set_button(&mut self, slot: usize, action: &ButtonAction) -> HubResult<bool> {
        if slot >= LABELS.len() {
            return Err(HubError::Invalid("HID++ button slot".into()));
        }
        let mut buttons = self.buttons();
        if &buttons[slot] == action {
            return Ok(false);
        }
        let encoded = encode(action)?;
        buttons[slot] = action.clone();
        if !buttons.contains(&ButtonAction::Mouse(MouseFn::Left)) {
            return Err(HubError::Invalid(
                "左クリックの割り当てを1つ残してください".into(),
            ));
        }
        let offset = BUTTONS + slot * BUTTON_BYTES;
        self.bytes[offset..offset + BUTTON_BYTES].copy_from_slice(&encoded);
        Ok(true)
    }

    pub(super) fn set_interval(&mut self, ms: u8) -> bool {
        let changed = self.bytes[0] != ms;
        self.bytes[0] = ms;
        changed
    }

    pub(super) fn write(mut self, channel: &Channel) -> HubResult<()> {
        let checksum = crc(&self.bytes[..CHECKSUM]).to_be_bytes();
        self.bytes[CHECKSUM..].copy_from_slice(&checksum);
        let [sh, sl] = self.sector.to_be_bytes();
        request(channel, self.feature, 6, &[sh, sl, 0, 0, 0, SIZE as u8])?;
        for chunk in self.bytes.chunks(16) {
            request(channel, self.feature, 7, chunk)?;
        }
        request(channel, self.feature, 8, &[])?;
        if read_sector(channel, self.feature, self.sector)? != self.bytes {
            return Err(HubError::Protocol(
                "HID++ profile write verification failed".into(),
            ));
        }
        Ok(())
    }
}

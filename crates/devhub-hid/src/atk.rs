//! ATK HUB (hub.atk.pro) 系マウス — 16B report-8 + EEPROM プロトコル。
//!
//! 公式 ATK HUB (hub-v3) の解析 + F1 V3 ULTIMATE+ (373b:12a7) 実機で検証。
//! ワイヤ形式は mouse.fit 系と同じ系列:
//!
//! - 設定コレクション usage_page 0xFF02 / usage 2、output/input report id 8。
//! - 要求  `[cmd, 0, addrHi, addrLo, len, data…]` (16B、17 バイト目に cksum)
//! - 応答  `[rid=8, cmd echo, status, addrHi, addrLo, len, data…, cksum]` (17B)
//! - cksum = `0x55 - (全バイト和 & 0xff)` (17B 合計 ≡ 0x55)
//! - EEPROM: 読み cmd8、書き cmd7 (10B チャンク)。スカラは (v, 0x55-v) ペア。
//!
//! EEPROM マップ (実機 + 公式 JS `y.Address`):
//! ReportRate ブロック @0 (10B: rate,maxDpi,currentDpi,bhop,btnOpMode + 各crc)、
//! LOD @10、DPI色 @44 (4B×8)、キー割当 @96 (4B×16)、MotionSync @171、
//! CloseLedTime @173、直線補正 @175、リップル @177、Sensor系 @181/183/185、
//! RfTxTime @187、WheelDebounce @227、Shortcut @256 (32B×16)、
//! Macro @768 (384B×16)、PAW3955 DPI @6912 (6B×8)、DynamicSensitivity @6960。
//!
//! その他 cmd: 3=online, 4=battery, 5/6=pair, 9=factory, 14/15=configID,
//! 16=CID/MID (F1 V3 = "1,62"), 18=FW, 22/23=長距離, 29=dongle FW。
//! CID=1 系は全機種この 16B 方式と思われる (CID=2 系は別世代)。

use devhub_core::backend::*;
use devhub_core::model::*;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

const RID: u8 = 8;
const CHUNK: u16 = 10;

#[allow(dead_code)]
mod ee {
    pub const REPORT_RATE: u16 = 0;
    pub const MAX_DPI_STAGE: u16 = 2;
    pub const CURRENT_DPI: u16 = 4;
    pub const LOD: u16 = 10;
    pub const DPI_COLOR: u16 = 44;
    pub const KEY_FUNCTION: u16 = 96;
    pub const DEBOUNCE: u16 = 169;
    pub const MOTION_SYNC: u16 = 171;
    /// スリープ時間 — 公式 closeLedTime。wire 値は 10 秒単位 (6 = 60s = 1分)。
    pub const CLOSE_LED: u16 = 173;
    pub const ANGLE_SNAP: u16 = 175;
    pub const RIPPLE: u16 = 177;
    /// 移動中 LED 消灯 (moveCloseLights)。
    pub const MOVE_LED: u16 = 179;
    /// ボタン別ショートカット領域 — 32B × slot (slot index = 物理ボタン)。
    pub const SHORTCUT: u16 = 256;
    pub const WHEEL_DEBOUNCE: u16 = 227;
    pub const DPI_3955: u16 = 6912;
}

/// cmd16 応答 (data[0]=CID, data[1]=MID) → (表示名, model id)。
/// ドングル経由でも中継されるため、受信機名のままにしない。
fn mid_name(cid: u8, mid: u8) -> Option<(&'static str, &'static str)> {
    match (cid, mid) {
        (1, 62) => Some(("ATK F1 V3 ULTIMATE+", "atk-f1v3u")),
        _ => None,
    }
}

// ==================================================================== 構造

pub struct AtkDriver {
    dev: Mutex<hidapi::HidDevice>,
    info: Mutex<DeviceInfo>,
    caps: Capabilities,
    events: Option<std::sync::Arc<dyn Fn(HubEvent) + Send + Sync>>,
    /// CID/MID で機種確定済みか (ドングル経由で休眠中は未確定のまま)。
    detected: Mutex<bool>,
}

pub fn open(
    dev: hidapi::HidDevice,
    info: DeviceInfo,
    events: Option<std::sync::Arc<dyn Fn(HubEvent) + Send + Sync>>,
) -> HubResult<AtkDriver> {
    dev.set_blocking_mode(false).ok();
    let d = AtkDriver {
        dev: Mutex::new(dev),
        info: Mutex::new(info),
        caps: build_caps(),
        events,
        detected: Mutex::new(false),
    };
    // CID/MID で機種確定 (応答があれば名を差し替える)。
    d.maybe_detect();
    Ok(d)
}

impl AtkDriver {
    /// `info` の変更を UI へ通知。
    fn emit_info(&self, i: DeviceInfo) {
        if let Some(ev) = &self.events {
            ev(HubEvent::Info(i.id.clone(), Box::new(i)));
        }
    }

    /// CID/MID 未確定なら cmd16 を試し、取れれば機種名を差し替える。
    /// ドングル経由でマウス休眠中は失敗するので、覚醒後の呼び出しで確定する。
    fn maybe_detect(&self) {
        if *self.detected.lock().unwrap_or_else(|p| p.into_inner()) {
            return;
        }
        let Ok(r) = self.cmd_raw(16, 0, &[]) else {
            return;
        };
        let (cid, mid) = (
            r.get(6).copied().unwrap_or(0),
            r.get(7).copied().unwrap_or(0),
        );
        if cid == 0 && mid == 0 {
            return;
        }
        *self.detected.lock().unwrap_or_else(|p| p.into_inner()) = true;
        let mut i = self.info.lock().unwrap_or_else(|p| p.into_inner());
        let mut dirty = false;
        if let Some((name, model)) = mid_name(cid, mid) {
            if i.name != name {
                i.name = name.into();
                i.model = model.into();
                dirty = true;
            }
        }
        if dirty {
            let info = i.clone();
            drop(i);
            self.emit_info(info);
        }
    }

    /// 休眠フラグの更新 (ドングル経由のみ意味を持つ)。
    fn set_asleep(&self, asleep: bool) {
        let mut i = self.info.lock().unwrap_or_else(|p| p.into_inner());
        if i.transport != Transport::Wireless24G || i.asleep == asleep {
            return;
        }
        i.asleep = asleep;
        let info = i.clone();
        drop(i);
        self.emit_info(info);
    }

    /// 16B コマンド → 17B 応答 (cmd echo を拾う。最大 3 回)。
    fn cmd_raw(&self, cmd: u8, addr: u16, data: &[u8]) -> HubResult<Vec<u8>> {
        let mut f = [0u8; 17];
        f[0] = RID;
        f[1] = cmd;
        f[3] = (addr >> 8) as u8;
        f[4] = addr as u8;
        f[5] = data.len().min(10) as u8;
        f[6..6 + data.len().min(10)].copy_from_slice(&data[..data.len().min(10)]);
        let s: u32 = f[..16].iter().map(|&b| b as u32).sum();
        f[16] = 85u32.wrapping_sub(s) as u8;
        let dev = self.dev.lock().unwrap_or_else(|p| p.into_inner());
        for _ in 0..3 {
            dev.write(&f)
                .map_err(|e| HubError::Io(format!("atk write: {e}")))?;
            let deadline = Instant::now() + Duration::from_millis(400);
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let mut buf = [0u8; 64];
                match dev.read_timeout(&mut buf, left.as_millis().min(120) as i32) {
                    Ok(n) if n >= 17 && buf[1] == cmd => {
                        self.set_asleep(false);
                        return Ok(buf[..17].to_vec());
                    }
                    _ => continue,
                }
            }
            sleep(Duration::from_millis(20));
        }
        // ドングル経由で応答が無い = マウス休眠中とみなす。
        self.set_asleep(true);
        Err(HubError::Protocol("atk: response timeout".into()))
    }

    /// EEPROM 読み出し (10B チャンク、内部結合)。データは resp[6..]。
    fn ee_read(&self, addr: u16, len: u16) -> HubResult<Vec<u8>> {
        let mut out = Vec::with_capacity(len as usize);
        let mut a = addr;
        while (out.len() as u16) < len {
            let want = (len - out.len() as u16).min(CHUNK) as u8;
            // GetEEPROM: len は要求の [4] (=data.len()=0 ではなく len フィールド)。
            let mut f = [0u8; 17];
            f[0] = RID;
            f[1] = 8;
            f[3] = (a >> 8) as u8;
            f[4] = a as u8;
            f[5] = want;
            let s: u32 = f[..16].iter().map(|&b| b as u32).sum();
            f[16] = 85u32.wrapping_sub(s) as u8;
            let dev = self.dev.lock().unwrap_or_else(|p| p.into_inner());
            let mut got = None;
            for _ in 0..3 {
                dev.write(&f)
                    .map_err(|e| HubError::Io(format!("atk write: {e}")))?;
                let deadline = Instant::now() + Duration::from_millis(400);
                loop {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    let mut buf = [0u8; 64];
                    match dev.read_timeout(&mut buf, left.as_millis().min(120) as i32) {
                        Ok(n) if n >= 17 && buf[1] == 8 => {
                            got = Some(buf);
                            break;
                        }
                        _ => continue,
                    }
                }
                if got.is_some() {
                    break;
                }
                sleep(Duration::from_millis(20));
            }
            drop(dev);
            let buf = got.ok_or_else(|| HubError::Protocol("atk: ee timeout".into()))?;
            let n = (buf[5] as usize).min(10).min(want as usize);
            out.extend_from_slice(&buf[6..6 + n]);
            if n < want as usize {
                break;
            }
            a += want as u16;
        }
        Ok(out)
    }

    /// EEPROM 書き込み (SetEEPROM cmd7、10B チャンク)。
    fn ee_write(&self, addr: u16, data: &[u8]) -> HubResult<()> {
        for (i, chunk) in data.chunks(CHUNK as usize).enumerate() {
            self.cmd_raw(7, addr + (i * CHUNK as usize) as u16, chunk)?;
            sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    /// スカラ (v, 0x55-v) ペア書き込み。
    fn ee_write_val(&self, addr: u16, val: u8) -> HubResult<()> {
        self.ee_write(addr, &[val, 85u8.wrapping_sub(val)])
    }

    /// スカラ読み出し。ペア検証付き。
    fn ee_u8(&self, addr: u16) -> HubResult<u8> {
        let b = self.ee_read(addr, 2)?;
        if (b[0] as u16 + b[1] as u16) & 0xff == 85 {
            Ok(b[0])
        } else {
            Err(HubError::Protocol(format!("eeprom @{addr}: bad pair")))
        }
    }

    /// @0 の 10B ブロックを読み、byte idx (=偶数 offset) を差し替えて書き戻す。
    fn cfg_byte(&self, idx: usize, val: u8) -> HubResult<()> {
        let mut b = self.ee_read(0, 10)?;
        b[idx] = val;
        b[idx + 1] = 85u8.wrapping_sub(val);
        self.ee_write(0, &b)
    }

    fn crc(b: &[u8]) -> u8 {
        85u8.wrapping_sub(b[..b.len() - 1].iter().map(|&x| x as u32).sum::<u32>() as u8)
    }

    /// 3955 DPI レコード (6B): [xLo,xHi,yLo,yHi,flags,crc]、raw+1 = dpi。
    fn parse_stage(b: &[u8]) -> (u32, u32) {
        let f = b[4] as u32;
        let mut x = b[0] as u32 | ((b[1] as u32) << 8) | (((f >> 2) & 3) << 16);
        let mut y = b[2] as u32 | ((b[3] as u32) << 8) | ((f >> 6) << 16);
        x += 1;
        y += 1;
        if f & 1 == 1 {
            x *= 2;
        }
        if f & 2 == 2 {
            y *= 2;
        }
        (x, y)
    }

    fn encode_stage(x: u32, y: u32) -> Vec<u8> {
        let l = x.saturating_sub(1);
        let s = y.saturating_sub(1);
        let mut b = vec![
            (l & 0xff) as u8,
            ((l >> 8) & 0xff) as u8,
            (s & 0xff) as u8,
            ((s >> 8) & 0xff) as u8,
            ((((l >> 16) & 3) << 2) | (((s >> 16) & 3) << 6)) as u8,
            0,
        ];
        b[5] = Self::crc(&b);
        b
    }

    /// KeyFunction @96 4B レコード → ButtonAction。type5=ShortcutKey は
    /// ボタンごとの領域 (@256+slot*32) を別途読む。
    fn decode_keyfn(&self, slot: usize, b: &[u8]) -> ButtonAction {
        match b[0] {
            1 => ButtonAction::Mouse(match b[1] {
                2 => MouseFn::Right,
                4 => MouseFn::Middle,
                8 => MouseFn::Back,
                16 => MouseFn::Forward,
                _ => MouseFn::Left,
            }),
            2 => ButtonAction::Dpi(match b[1] {
                2 => DpiFn::Up,
                3 => DpiFn::Down,
                _ => DpiFn::Cycle,
            }),
            4 if b[1] == 100 => ButtonAction::Mouse(MouseFn::DoubleClick),
            4 => ButtonAction::Fire {
                button: MouseFn::Left,
                count: b[2],
                interval_ms: b[1] as u16,
            },
            6 => ButtonAction::Macro {
                slot: b[1].saturating_sub(1),
                run: match b[2] {
                    253 => MacroRun::Toggle,
                    254 => MacroRun::WhileHeld,
                    n => MacroRun::Times(n as u16),
                },
            },
            5 => self.read_shortcut(slot).unwrap_or(ButtonAction::Disabled),
            7 => ButtonAction::PollingCycle,
            10 => ButtonAction::Dpi(DpiFn::Shift(((b[1] as u32) + 1) * 50)),
            11 => ButtonAction::Mouse(if b[1] == 2 {
                MouseFn::ScrollDown
            } else {
                MouseFn::ScrollUp
            }),
            _ => ButtonAction::Disabled,
        }
    }

    /// ShortcutKey 領域 (@256 + slot*32) を読む。
    /// 形式: [count, entry…, sum] — entry = {keyState, v1, v2}、
    /// keyState = 0x80(押下)|0x40(解放) + keyType (1=Keyboard 2=Fire 3=Combination 4=Macro)。
    /// sum = 0x55 - (count + 全 entry bytes)。
    fn read_shortcut(&self, slot: usize) -> Option<ButtonAction> {
        let b = self.ee_read(ee::SHORTCUT + (slot * 32) as u16, 32).ok()?;
        let n = *b.first()? as usize;
        if n == 0 || n > 10 {
            return None;
        }
        // 最初の「押下」エントリのキーを代表表示に使う。
        for i in 0..n {
            let e = &b[1 + i * 3..1 + i * 3 + 3];
            let (state, ty) = (e[0], e[0] & 0x3f);
            if state & 0x80 == 0 || ty != 1 {
                continue;
            }
            // keyType=Keyboard: v1 = 使用コード (公式は独自キー表)。修飾は v2 側。
            return Some(ButtonAction::Key(KeyStroke {
                mods: e[2],
                usage: e[1] as u16,
            }));
        }
        None
    }

    /// ButtonAction::Key → ShortcutKey 領域 (押下+解放の 2 エントリ)。
    fn encode_shortcut(&self, k: &KeyStroke) -> Vec<u8> {
        let mut b = vec![
            2,
            0x80 | 1,
            (k.usage & 0xff) as u8,
            k.mods,
            0x40 | 1,
            (k.usage & 0xff) as u8,
            k.mods,
        ];
        let s: u8 = b.iter().fold(0u8, |a, &x| a.wrapping_add(x));
        b.push(85u8.wrapping_sub(s));
        b.resize(32, 0);
        b
    }

    /// ButtonAction → KeyFunction 4B レコード。
    fn encode_keyfn(&self, a: &ButtonAction) -> HubResult<[u8; 4]> {
        use ButtonAction::*;
        let rec = |ty: u8, ph: u8, pl: u8| {
            let mut r = [ty, ph, pl, 0];
            r[3] = Self::crc(&r);
            r
        };
        Ok(match a {
            Disabled => rec(0, 0, 0),
            Mouse(MouseFn::DoubleClick) => rec(4, 100, 2),
            Mouse(MouseFn::ScrollUp) => rec(11, 1, 0),
            Mouse(MouseFn::ScrollDown) => rec(11, 2, 0),
            Mouse(f) => {
                let mask = match f {
                    MouseFn::Left => 1,
                    MouseFn::Right => 2,
                    MouseFn::Middle => 4,
                    MouseFn::Back => 8,
                    MouseFn::Forward => 16,
                    _ => 1,
                };
                rec(1, mask, 0)
            }
            Dpi(DpiFn::Cycle) => rec(2, 1, 0),
            Dpi(DpiFn::Up) => rec(2, 2, 0),
            Dpi(DpiFn::Down) => rec(2, 3, 0),
            Dpi(DpiFn::Shift(d)) => rec(10, 0, ((*d / 50).max(1) - 1) as u8),
            Fire {
                count, interval_ms, ..
            } => rec(4, (*interval_ms).min(255) as u8, *count),
            Macro { slot: s, run } => {
                let t = match run {
                    MacroRun::Times(n) => (*n).min(252) as u8,
                    MacroRun::WhileHeld => 254,
                    MacroRun::Toggle => 253,
                };
                rec(6, *s + 1, t)
            }
            PollingCycle => rec(7, 0, 0),
            // ShortcutKey — 本体の割当は別領域 (@256+slot*32) に書く。
            Key(_) => rec(5, 0, 0),
            Media(_) => return Err(HubError::Unsupported("media button")),
            ProfileCycle => return Err(HubError::Unsupported("profile cycle")),
            ScrollModeToggle => return Err(HubError::Unsupported("scroll-mode toggle")),
            Unsupported { .. } => return Err(HubError::Unsupported("raw action")),
        })
    }
}

// ==================================================================== caps

fn opts(v: &[(i64, &str)]) -> Vec<ChoiceOpt> {
    v.iter().map(|(a, b)| ChoiceOpt::new(*a, *b)).collect()
}

fn build_caps() -> Capabilities {
    use section::*;
    Capabilities {
        settings: vec![
            SettingSpec::choice(
                "polling_wireless",
                PERFORMANCE,
                "ポーリングレート",
                opts(&[
                    (125, "125"),
                    (250, "250"),
                    (500, "500"),
                    (1000, "1K"),
                    (2000, "2K"),
                    (4000, "4K"),
                    (8000, "8K"),
                ]),
            )
            .help("マウスの報告頻度 (Hz)"),
            SettingSpec::choice(
                "lod",
                SENSOR,
                "リフトオフ距離",
                opts(&[
                    (7, "0.7mm"),
                    (8, "0.8mm"),
                    (9, "0.9mm"),
                    (10, "1.0mm"),
                    (11, "1.1mm"),
                    (12, "1.2mm"),
                    (13, "1.3mm"),
                    (14, "1.4mm"),
                    (15, "1.5mm"),
                    (16, "1.6mm"),
                    (17, "1.7mm"),
                ]),
            )
            .help("持ち上げて反応しなくなる高さ — PAW3955 は 0.7–1.7mm・0.1mm 刻み"),
            SettingSpec::toggle("motion_sync", SENSOR, "モーション同期")
                .help("ポインタとマウスの動きを同期させ、軌跡を滑らかにします"),
            SettingSpec::toggle("angle_snap", SENSOR, "直線補正")
                .help("移動のブレを直線に補正します"),
            SettingSpec::toggle("ripple", SENSOR, "リップル制御")
                .help("高DPI時の細かい揺れを抑えます"),
            SettingSpec::slider("wheel_debounce", CLICK, "ホイール遅延", 0., 60., 1., "ms")
                .help("ホイールの入力遅延 (実機初期値 30)"),
            SettingSpec::slider("debounce", CLICK, "クリックデバウンス", 0., 32., 1., "ms")
                .help("ボタンのチャタリング防止 (@169 stabilizationTime)"),
            SettingSpec::choice(
                "sleep_min",
                POWER,
                "スリープまでの時間",
                opts(&[
                    (1, "1 分"),
                    (2, "2 分"),
                    (3, "3 分"),
                    (5, "5 分"),
                    (10, "10 分"),
                    (30, "30 分"),
                    (60, "60 分"),
                ]),
            )
            .help("無操作でスリープするまでの時間 (@173、10秒単位で保持)"),
            SettingSpec::toggle("move_close_led", LIGHTING, "移動中 LED 消灯")
                .help("操作している間は LED を消します (@179)"),
            SettingSpec::slider("bhop", CLICK, "ホイール誤操作防止", 0., 25., 1., "")
                .help("スクロール誤検知の抑止時間 (0=無効、公式は time/10 で保持)"),
        ],
        actions: vec![
            ActionSpec::new("factory_reset", SYSTEM, "工場出荷状態に戻す")
                .help("全設定を初期値に戻します (cmd9)")
                .danger("このデバイスの全設定が消去されます"),
        ],
        profiles: Some(ProfileCaps {
            count: 4,
            renamable: false,
        }),
        mouse: Some(MouseCaps {
            dpi_min: 50,
            commit_dpi_on_release: false,
            dpi_max: 42000,
            dpi_step: 1,
            min_stages: 1,
            max_stages: 8,
            xy_split: true,
            stage_colors: true,
            stage_toggle: false,
            buttons: [
                "Left",
                "Right",
                "ホイールクリック",
                "サイド (戻る)",
                "サイド (進む)",
            ]
            .iter()
            .map(|n| ButtonSlot { name: (*n).into() })
            .collect(),
            button_kinds: vec![
                ButtonActionKind::Mouse,
                ButtonActionKind::Key,
                ButtonActionKind::Dpi,
                ButtonActionKind::Fire,
                ButtonActionKind::Macro,
                ButtonActionKind::PollingCycle,
                ButtonActionKind::Disabled,
            ],
            button_options: None,
            require_left_click: true,
            macros: None,
            live_buttons: vec![],
            adaptive_dpi: None,
        }),
        keyboard: None,
        lighting: vec![],
        battery: true,
        peq: None,
        firmware_feed: None,
    }
}

// ==================================================================== DeviceDriver

impl DeviceDriver for AtkDriver {
    fn info(&self) -> DeviceInfo {
        self.maybe_detect();
        let mut i = self.info.lock().unwrap_or_else(|p| p.into_inner()).clone();
        // ドングル経由: cmd29 はドングル自身が応答 (マウス休眠中も返る)。
        if i.transport == Transport::Wireless24G {
            if let Ok(r) = self.cmd_raw(29, 0, &[]) {
                if r[2] != 1 && (r[6] != 0 || r[7] != 0) {
                    i.dongle_firmware = Some(format!("{}.{}", r[6], r[7]));
                }
            }
        }
        // cmd18 が通らなければマウスは休眠中。
        match self.cmd_raw(18, 0, &[]) {
            Ok(r) => {
                i.asleep = false;
                if r[6] != 0 || r[7] != 0 {
                    i.firmware = Some(format!("{}.{}", r[6], r[7]));
                }
            }
            Err(_) => {
                i.asleep = i.transport == Transport::Wireless24G;
            }
        }
        if let Ok(b) = self.battery() {
            i.battery = Some(b);
            i.asleep = false;
        }
        i.supported = true;
        i
    }

    fn caps(&self) -> Capabilities {
        self.caps.clone()
    }

    fn read(&self) -> HubResult<DeviceState> {
        self.maybe_detect();
        // @0 ブロック: rate, stage数, active。
        let blk = self.ee_read(0, 10)?;
        let rate_code = blk[0];
        let stage_n = (blk[2] as usize).clamp(1, 8);
        let active = (blk[4] as usize).min(stage_n - 1);
        let stages_raw = self.ee_read(ee::DPI_3955, (stage_n * 6) as u16)?;
        let colors = self.ee_read(ee::DPI_COLOR, (stage_n * 4) as u16)?;
        let mut stages = vec![];
        for i in 0..stage_n {
            let rec = &stages_raw[i * 6..i * 6 + 6];
            let (x, y) = Self::parse_stage(rec);
            let c = &colors[i * 4..i * 4 + 4];
            stages.push(DpiStage {
                x,
                y: if y == x { None } else { Some(y) },
                color: Rgb {
                    r: c[0],
                    g: c[1],
                    b: c[2],
                },
                enabled: true,
            });
        }
        // ボタン key0..4 @96。
        let keys = self.ee_read(ee::KEY_FUNCTION, 20)?;
        let buttons = (0..5)
            .map(|i| self.decode_keyfn(i, &keys[i * 4..i * 4 + 4]))
            .collect();

        let mut settings = BTreeMap::new();
        let rate = match rate_code {
            8 => 125,
            4 => 250,
            2 => 500,
            1 => 1000,
            16 => 2000,
            32 => 4000,
            64 => 8000,
            _ => 1000,
        };
        settings.insert("polling_wireless".into(), SettingValue::Choice(rate));
        if let Ok(v) = self.ee_u8(ee::LOD) {
            // @10 = LOD レベル 1–11 → 0.7–1.7mm (PAW3955 "Ultimate" 系)。
            let mm10 = (v.clamp(1, 11) + 6) as i64;
            settings.insert("lod".into(), SettingValue::Choice(mm10));
        }
        for (addr, id) in [
            (ee::MOTION_SYNC, "motion_sync"),
            (ee::ANGLE_SNAP, "angle_snap"),
            (ee::RIPPLE, "ripple"),
        ] {
            if let Ok(v) = self.ee_u8(addr) {
                settings.insert(id.into(), SettingValue::Bool(v != 0));
            }
        }
        if let Ok(v) = self.ee_u8(ee::WHEEL_DEBOUNCE) {
            settings.insert("wheel_debounce".into(), SettingValue::Num(v as f32));
        }
        if let Ok(v) = self.ee_u8(ee::DEBOUNCE) {
            settings.insert("debounce".into(), SettingValue::Num(v as f32));
        }
        if let Ok(v) = self.ee_u8(ee::CLOSE_LED) {
            // wire = 10 秒単位 → 分に変換 (6 → 1分)。
            let min = ((v as u32 * 10) / 60).max(1) as i64;
            settings.insert("sleep_min".into(), SettingValue::Choice(min));
        }
        if let Ok(v) = self.ee_u8(ee::MOVE_LED) {
            settings.insert("move_close_led".into(), SettingValue::Bool(v != 0));
        }
        // @0 byte6 = bhop (ホイール誤操作防止、time/10)。
        settings.insert("bhop".into(), SettingValue::Num(blk[6] as f32));
        // 注: cmd22/23 の長距離モードはドングル側機能 — 有線では cmd22 は ACK するが
        // cmd23 は常に 0 を返す (実機確認) ので設定には出さない。
        // オンボードプロファイル: cmd14 GetCurrentConfig → data[0] = index。
        let profile = self
            .cmd_raw(14, 0, &[])
            .ok()
            .and_then(|r| r.get(6).copied())
            .unwrap_or(0);
        Ok(DeviceState {
            profile: ProfileState {
                active: profile.min(3),
                names: vec![],
            },
            settings,
            mouse: Some(MouseState {
                stages,
                active_stage: active,
                buttons,
                macros: vec![],
                adaptive: None,
            }),
            keyboard: None,
            lighting: BTreeMap::new(),
            peq: None,
        })
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        match change {
            Change::Setting { id, value } => match id.as_str() {
                "polling_wireless" => {
                    let code = match value.as_choice() {
                        125 => 8,
                        250 => 4,
                        500 => 2,
                        1000 => 1,
                        2000 => 16,
                        4000 => 32,
                        8000 => 64,
                        _ => return Err(HubError::Invalid("polling rate".into())),
                    };
                    self.cfg_byte(0, code)
                }
                "lod" => self.ee_write_val(ee::LOD, (value.as_choice().clamp(7, 17) - 6) as u8),
                "motion_sync" => self.ee_write_val(ee::MOTION_SYNC, value.as_bool() as u8),
                "angle_snap" => self.ee_write_val(ee::ANGLE_SNAP, value.as_bool() as u8),
                "ripple" => self.ee_write_val(ee::RIPPLE, value.as_bool() as u8),
                "wheel_debounce" => {
                    self.ee_write_val(ee::WHEEL_DEBOUNCE, value.as_num().clamp(0., 255.) as u8)
                }
                "debounce" => self.ee_write_val(ee::DEBOUNCE, value.as_num().clamp(0., 255.) as u8),
                "sleep_min" => {
                    // 分 → 10 秒単位 (1分→6)。
                    self.ee_write_val(ee::CLOSE_LED, (value.as_choice().clamp(1, 42) * 6) as u8)
                }
                "move_close_led" => self.ee_write_val(ee::MOVE_LED, value.as_bool() as u8),
                "bhop" => self.cfg_byte(6, value.as_num().clamp(0., 25.) as u8),
                _ => Err(HubError::Unsupported("setting")),
            },
            Change::DpiStages { stages, active } => {
                let n = stages.len().clamp(1, 8);
                self.ee_write_val(ee::MAX_DPI_STAGE, n as u8)?;
                self.ee_write_val(ee::CURRENT_DPI, (*active).min(n - 1) as u8)?;
                for (i, s) in stages[..n].iter().enumerate() {
                    self.ee_write(
                        ee::DPI_3955 + (i * 6) as u16,
                        &Self::encode_stage(s.x, s.y.unwrap_or(s.x)),
                    )?;
                    let mut c = vec![s.color.r, s.color.g, s.color.b, 0];
                    c[3] = Self::crc(&c);
                    self.ee_write(ee::DPI_COLOR + (i * 4) as u16, &c)?;
                }
                Ok(())
            }
            Change::ActiveStage(a) => self.ee_write_val(ee::CURRENT_DPI, *a as u8),
            Change::Button { slot, action } => {
                if *slot >= 5 {
                    return Err(HubError::Invalid("button slot".into()));
                }
                let rec = self.encode_keyfn(action)?;
                // ShortcutKey は本体領域を先に書いてから keyfn を指す。
                if let ButtonAction::Key(k) = action {
                    self.ee_write(ee::SHORTCUT + (*slot * 32) as u16, &self.encode_shortcut(k))?;
                }
                self.ee_write(ee::KEY_FUNCTION + (*slot * 4) as u16, &rec)
            }
            Change::Profile(i) => self.cmd_raw(15, 0, &[*i]).map(|_| ()),
            _ => Err(HubError::Unsupported("change")),
        }
    }

    fn run_action(&self, id: &str, _file: Option<&std::path::Path>) -> HubResult<ActionOutcome> {
        match id {
            "factory_reset" => {
                self.cmd_raw(9, 0, &[])?;
                Ok(ActionOutcome {
                    message: Some("工場出荷状態に戻しました".into()),
                    reload: true,
                })
            }
            _ => Err(HubError::Unsupported("action")),
        }
    }

    fn battery(&self) -> HubResult<Battery> {
        // cmd4: data[0]=percent, data[1]=chargingStatus。
        let r = self.cmd_raw(4, 0, &[])?;
        Ok(Battery {
            percent: Some(r.get(6).copied().unwrap_or(0).min(100)),
            charging: r.get(7).copied().unwrap_or(0) != 0,
        })
    }
}

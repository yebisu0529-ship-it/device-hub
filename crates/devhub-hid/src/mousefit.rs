//! mouse.fit プロトコル (www.mouse.fit / G-Wolves 等) — 新プロトコル機種専用。
//!
//! 対象機種 (HTS Plus Pro / HTX Ultra / 8K ドングル) はすべて `IsNewProtocol=1`
//! (docs/PROTOCOL_NOTES.md, env-models.json)。旧 report-8 EEPROM 経路は実装しない。
//!
//! # トランスポート
//! - 64B feature report のみ (`send_feature_report(0)` / `get_feature_report(0)`)。
//! - 要求: `[2]=devID, [3]=group, [4]=sub-family, [5]=cmd, [6..]=args`。
//!   読みは `cmd = 0x80|n`、書きは `cmd = n`。`[3]` は可変 (ボタン/マクロ系は `5+ctxLen` 等)。
//! - 応答: `L[1]=0xA1` (ACK)。レスポンスは要求を 1 バイト右にずらした形でエコーし、
//!   データは「7 + 要求引数バイト数」の位置に来る:
//!   - 引数なし要求 (FW/battery/PID) → データは L[7..]
//!   - profile を `[6]` に置く要求 → データは L[8..]
//!   - stage-info のように `[6]=profile,[7]=N` → L[8]=N, レコードは L[9..]
//! - `hidIndex`: 応答ペイロード先頭が 0xA0 以上なら hi=1 (L[i]=buf[i])、
//!   未満なら hi=0 (L[i]=buf[1+i])。先頭にゴミが載る実機差を吸収する JS 由来の規則。
//!
//! # プロファイル / ステージは 1-based
//! getProfileID / setActiveDPI は 1 始まり。v2 側の `usize` へは -1 する。

use crate::transport::Channel;
use devhub_core::backend::*;
use devhub_core::model::*;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

const ACK: u8 = 0xA1;
const MAX_STAGES: usize = 7;

// ==================================================================== 機種表

/// env-models.json 由来の機種差。
pub struct MfModel {
    /// true = GW_056 系 (XVI=1): ボタンは ie 型 + z-id。false = JM 系: Ya 型 + slot index。
    pub xvi: bool,
    pub dpi_max: u32,
    pub dpi_step: u32,
    pub max_stages: usize,
    /// LOD の選択肢 (display mm ×10 の i64 値とラベル)。
    pub lod_choices: &'static [(i64, &'static str)],
    /// JM 系は LOD を 1-based index で書く。GW 系は実値 (0.7→0x87)。
    pub lod_is_index: bool,
    pub spdt: bool,
    pub liftoff_debounce: bool,
    /// ポーリング候補 (Hz)。
    pub polling: &'static [u32],
}

const LOD_JM: &[(i64, &str)] = &[
    (7, "0.7mm"),
    (9, "0.9mm"),
    (12, "1.2mm"),
    (14, "1.4mm"),
    (16, "1.6mm"),
];
const LOD_GW: &[(i64, &str)] = &[(7, "0.7mm"), (10, "1.0mm"), (20, "2.0mm")];
const POLL_8K: &[u32] = &[125, 250, 500, 1000, 2000, 4000, 8000];

/// HTS Plus Pro — JM_052_3955 (XVI=0, 3955, 磁気スイッチだがトリガー API は stub)。
const HTS_PLUS_PRO: MfModel = MfModel {
    xvi: false,
    dpi_max: 40_000,
    dpi_step: 1,
    max_stages: 6,
    lod_choices: LOD_JM,
    lod_is_index: true,
    spdt: false,
    liftoff_debounce: false,
    polling: POLL_8K,
};
/// HTX Ultra (GW_056, XVI=1)。
const HTX_ULTRA_GW: MfModel = MfModel {
    xvi: true,
    dpi_max: 30_000,
    dpi_step: 50,
    max_stages: 7,
    lod_choices: LOD_GW,
    lod_is_index: false,
    spdt: true,
    liftoff_debounce: true,
    polling: POLL_8K,
};
/// HTX Ultra 3950 版 (JM_056_3950, XVI=0)。
const HTX_ULTRA_3950: MfModel = MfModel {
    xvi: false,
    dpi_max: 32_000,
    dpi_step: 50,
    max_stages: 7,
    lod_choices: LOD_JM,
    lod_is_index: true,
    spdt: true,
    liftoff_debounce: false,
    polling: POLL_8K,
};
/// HTX Ultra 3955 版 (JM_056_3955, XVI=0)。
const HTX_ULTRA_3955: MfModel = MfModel {
    xvi: false,
    dpi_max: 40_000,
    dpi_step: 1,
    max_stages: 7,
    lod_choices: LOD_JM,
    lod_is_index: true,
    spdt: true,
    liftoff_debounce: false,
    polling: POLL_8K,
};

fn model_for(model: &str, pid: u16) -> &'static MfModel {
    match (model, pid) {
        ("hts-plus-pro", _) => &HTS_PLUS_PRO,
        ("htx-ultra", _) => &HTX_ULTRA_GW,
        ("htx-ultra-3950", _) => &HTX_ULTRA_3950,
        ("htx-ultra-3955", _) => &HTX_ULTRA_3955,
        // 8K ドングル単体は開いてから GetDevicePID_2 で実機を引く (open() が差し替える)。
        _ => &HTS_PLUS_PRO,
    }
}

/// ドングルが報告した内側 PID → registry model 名。
fn model_name_for_inner_pid(pid: u16) -> &'static str {
    match pid {
        0x5219 => "hts-plus-pro",
        0x5608 | 0x5617 => "htx-ultra",
        0x5618 => "htx-ultra-3950",
        0x5619 => "htx-ultra-3955",
        _ => "hts-plus-pro",
    }
}

// ==================================================================== 応答アクセス

/// feature report 応答を「論理位置」で読むラッパ。
/// JS の `o[i - hidIndex]` と同じ: `L[i]` = 論理位置 i のバイト。
fn logical(buf: &[u8; 65]) -> [u8; 64] {
    // hidapi は buf[0]=report ID(0), buf[1..]=payload。
    // payload[0] >= 0xA0 → hi=1 (L[i]=buf[base+i-1]); それ未満 → hi=0 (L[i]=buf[base+i])。
    // 環境によって payload が buf[0] 始まりのこともあるので buf[1]→buf[0] の順で判定。
    let (base, hi) = if buf[1] >= 0xA0 {
        (1usize, 1usize)
    } else if buf[0] >= 0xA0 {
        (0, 1)
    } else {
        (1, 0)
    };
    let mut l = [0u8; 64];
    for i in hi..64 {
        l[i] = buf[base + i - hi];
    }
    l
}

fn u16be(l: &[u8; 64], i: usize) -> u16 {
    ((l[i] as u16) << 8) | l[i + 1] as u16
}
fn u32be(l: &[u8; 64], i: usize) -> u32 {
    ((l[i] as u32) << 24) | ((l[i + 1] as u32) << 16) | ((l[i + 2] as u32) << 8) | l[i + 3] as u32
}

// ==================================================================== ドライバ

pub struct MousefitDriver {
    info: DeviceInfo,
    ch: Channel,
    model: &'static MfModel,
    caps: Capabilities,
    firmware: Mutex<Option<String>>,
    dongle_firmware: Mutex<Option<String>>,
}

pub fn open(dev: hidapi::HidDevice, info: DeviceInfo) -> HubResult<MousefitDriver> {
    let ch = Channel::open(dev, 65, 65);
    let mut info = info;
    // ドングル経由 (0x3854) のとき内側の実機 PID を聞いて機種を差し替える。
    let probe = MousefitDriver {
        caps: Capabilities::empty(),
        model: &HTS_PLUS_PRO,
        firmware: Mutex::new(None),
        dongle_firmware: Mutex::new(None),
        info: info.clone(),
        ch,
    };
    let (fw, inner_pid) = probe.firmware_version(2).unwrap_or((None, None));
    let dongle_fw = if info.transport == Transport::Wireless24G {
        probe.firmware_version(0).ok().and_then(|(f, _)| f)
    } else {
        None
    };
    let model = if info.pid == 0x3854 {
        let inner = inner_pid.unwrap_or(0x5219);
        info.model = model_name_for_inner_pid(inner).into();
        info.name = format!(
            "{} (wireless)",
            registry_name(info.model.as_str(), &info.name)
        );
        model_for(&info.model, inner)
    } else {
        model_for(&info.model, info.pid)
    };
    *probe.firmware.lock().unwrap_or_else(|p| p.into_inner()) = fw.clone();
    *probe
        .dongle_firmware
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = dongle_fw.clone();
    info.firmware = fw;
    info.dongle_firmware = dongle_fw;
    info.supported = true;
    let caps = build_caps(&info, model);
    Ok(MousefitDriver {
        info,
        ch: probe.ch,
        model,
        caps,
        firmware: probe.firmware,
        dongle_firmware: probe.dongle_firmware,
    })
}

fn registry_name(_model: &str, fallback: &str) -> String {
    fallback.trim_end_matches(" (wireless)").to_string()
}

impl MousefitDriver {
    // ---- 基本コマンド --------------------------------------------------
    /// 要求 64B → 応答の論理 64B。ACK (L[1]==0xA1) を確認。最大 3 回再送。
    fn cmd(&self, req: &[u8; 64]) -> HubResult<[u8; 64]> {
        let mut last = HubError::Protocol("no response".into());
        for _ in 0..3 {
            self.ch.send_feature(req)?;
            sleep(Duration::from_millis(40));
            match self.ch.get_feature() {
                Ok(buf) => {
                    let l = logical(&buf);
                    if l[1] == ACK || l[1] == 2 || l[1] == 0xA2 {
                        return Ok(l);
                    }
                    last = HubError::Protocol(format!("bad ack {:02x} (cmd {:02x})", l[1], req[5]));
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// dev/group/sub/cmd + args ([6..])。
    fn req(&self, dev: u8, group: u8, sub: u8, cmd: u8, args: &[u8]) -> HubResult<[u8; 64]> {
        let mut r = [0u8; 64];
        r[2] = dev;
        r[3] = group;
        r[4] = sub;
        r[5] = cmd;
        let n = args.len().min(58);
        r[6..6 + n].copy_from_slice(&args[..n]);
        self.cmd(&r)
    }

    /// profile スコープの読み (引数 1B) → L[8]。
    fn read8(&self, prof: u8, group: u8, sub: u8, cmd: u8) -> HubResult<u8> {
        let l = self.req(2, group, sub, cmd, &[prof])?;
        Ok(l[8])
    }
    /// profile スコープの書き (引数 profile + value)。
    fn write1(&self, prof: u8, group: u8, sub: u8, cmd: u8, v: u8) -> HubResult<()> {
        self.req(2, group, sub, cmd, &[prof, v])?;
        Ok(())
    }

    /// FW バージョンと内側 PID (resp L[7..11]=version, L[11..13]=pid)。
    fn firmware_version(&self, dev: u8) -> HubResult<(Option<String>, Option<u16>)> {
        let l = self.req(dev, 16, 0, 129, &[])?;
        if l[6] != 129 {
            return Ok((None, None));
        }
        let fw = format!("{}.{}.{}.{}", l[7], l[8], l[9], l[10]);
        let pid = u16be(&l, 11);
        Ok((Some(fw), (pid != 0).then_some(pid)))
    }

    fn profile(&self) -> HubResult<u8> {
        let l = self.req(2, 1, 0, 133, &[])?;
        Ok(l[7].max(1))
    }

    fn set_profile(&self, id: u8) -> HubResult<()> {
        self.req(2, 1, 0, 5, &[id])?;
        Ok(())
    }

    fn profile_count(&self) -> u8 {
        let l = match self.req(2, 1, 0, 134, &[]) {
            Ok(l) => l,
            Err(_) => return 1,
        };
        let v = l[7];
        // 値が個数かビットマスクか機種で揺れる → 8 超は popcount とみなす。
        if v == 0 {
            1
        } else if v <= 8 {
            v
        } else {
            v.count_ones() as u8
        }
        .max(1)
    }

    // ---- 個別機能 -------------------------------------------------------
    fn polling(&self, prof: u8) -> HubResult<u32> {
        let code = self.read8(prof, 2, 1, 128)?;
        Ok(poll_from_code(code))
    }
    fn set_polling(&self, prof: u8, hz: u32) -> HubResult<()> {
        self.write1(prof, 2, 1, 0, poll_to_code(hz)?)
    }

    fn dpi_stages(&self, prof: u8) -> HubResult<(usize, Vec<(u32, u32)>)> {
        let l = self.req(2, 10, 1, 129, &[prof, self.model.max_stages as u8])?;
        let n = (l[8] as usize).min(MAX_STAGES);
        let mut v = Vec::with_capacity(n);
        for i in 0..n {
            let b = 9 + i * 4;
            v.push((u16be(&l, b) as u32, u16be(&l, b + 2) as u32));
        }
        Ok((n, v))
    }
    fn write_dpi_stages(&self, prof: u8, stages: &[(u32, u32)]) -> HubResult<()> {
        let mut args = vec![prof, stages.len() as u8];
        for &(x, y) in stages {
            args.extend_from_slice(&(x as u16).to_be_bytes());
            args.extend_from_slice(&(y as u16).to_be_bytes());
        }
        self.req(2, 30, 1, 1, &args)?;
        Ok(())
    }
    fn active_stage(&self, prof: u8) -> HubResult<usize> {
        let v = self.read8(prof, 2, 1, 130)?;
        Ok(v.max(1) as usize - 1)
    }
    fn set_active_stage(&self, prof: u8, idx: usize) -> HubResult<()> {
        self.write1(prof, 2, 1, 2, (idx + 1) as u8)
    }
    fn dpi_colors(&self, prof: u8) -> HubResult<Vec<Rgb>> {
        let l = self.req(2, 22, 2, 129, &[prof])?;
        let mut v = Vec::with_capacity(MAX_STAGES);
        for i in 0..MAX_STAGES {
            let b = 8 + i * 3;
            v.push(Rgb::new(l[b], l[b + 1], l[b + 2]));
        }
        Ok(v)
    }
    fn set_dpi_colors(&self, prof: u8, colors: &[Rgb]) -> HubResult<()> {
        let mut args = vec![prof];
        for i in 0..MAX_STAGES {
            let c = colors
                .get(i)
                .copied()
                .unwrap_or_else(|| colors.last().copied().unwrap_or(Rgb::hex(0xffffff)));
            args.extend_from_slice(&[c.r, c.g, c.b]);
        }
        self.req(2, 22, 2, 1, &args)?;
        Ok(())
    }

    fn lod(&self, prof: u8) -> HubResult<u8> {
        self.read8(prof, 2, 1, 136)
    }
    fn set_lod(&self, prof: u8, wire: u8) -> HubResult<()> {
        self.write1(prof, 2, 1, 8, wire)
    }

    fn bool_get(&self, prof: u8, group: u8, sub: u8, cmd: u8) -> HubResult<bool> {
        Ok(self.read8(prof, group, sub, cmd)? == 1)
    }
    fn bool_set(&self, prof: u8, group: u8, sub: u8, cmd: u8, on: bool) -> HubResult<()> {
        self.write1(prof, group, sub, cmd, on as u8)
    }

    fn debounce(&self, prof: u8) -> HubResult<[u8; 3]> {
        let l = self.req(2, 5, 0, 136, &[prof])?;
        Ok([l[8], l[9], l[10]])
    }
    fn set_debounce(&self, prof: u8, v: &[u8; 3]) -> HubResult<()> {
        self.req(2, 5, 0, 8, &[prof, v[0], v[1], v[2]])?;
        Ok(())
    }

    fn scroll_scan(&self, prof: u8) -> HubResult<(u8, u8)> {
        let l = self.req(2, 3, 0, 156, &[prof])?;
        Ok((l[8], l[9]))
    }
    fn set_scroll_scan(&self, prof: u8, rate: u8, deb: u8) -> HubResult<()> {
        self.req(2, 3, 0, 28, &[prof, rate, deb])?;
        Ok(())
    }

    fn spdt(&self, prof: u8) -> HubResult<(bool, bool)> {
        let l = self.req(2, 3, 3, 131, &[prof])?;
        Ok((l[8] == 1, l[9] == 1))
    }
    fn set_spdt(&self, prof: u8, l_on: bool, r_on: bool) -> HubResult<()> {
        self.req(2, 3, 3, 3, &[prof, l_on as u8, r_on as u8])?;
        Ok(())
    }

    fn sleep_min(&self, prof: u8) -> HubResult<u16> {
        let l = self.req(2, 3, 0, 135, &[prof])?;
        Ok(u16be(&l, 8))
    }
    fn set_sleep_min(&self, prof: u8, min: u16) -> HubResult<()> {
        let [hi, lo] = min.to_be_bytes();
        self.req(2, 3, 0, 7, &[prof, hi, lo])?;
        Ok(())
    }

    // ---- ボタン --------------------------------------------------------
    /// GetButton: [7]=id, [9]=type問い合わせ(0), [10]=ctx len(0)。応答 L[10]=type, L[12..]=ctx。
    fn get_button(&self, prof: u8, id: u8) -> HubResult<(u8, Vec<u8>)> {
        let l = self.req(2, 5, 3, 128, &[prof, id, 0, 0, 0])?;
        let len = l[11] as usize; // 応答側の ctx 長 (echo+1 位置)
        let start = 12;
        let ctx: Vec<u8> = (0..len.min(20)).map(|i| l[start + i]).collect();
        Ok((l[10], ctx))
    }
    /// SetButton: [7]=id(slot/zid), [9]=type, [10]=ctx len, [11..]=ctx, [3]=5+len。
    fn set_button(&self, prof: u8, id: u8, ty: u8, ctx: &[u8]) -> HubResult<()> {
        let mut r = [0u8; 64];
        r[2] = 2;
        r[3] = 5 + ctx.len() as u8;
        r[4] = 3;
        r[5] = 0;
        r[6] = prof;
        r[7] = id;
        r[9] = ty;
        r[10] = ctx.len() as u8;
        r[11..11 + ctx.len()].copy_from_slice(ctx);
        self.cmd(&r)?;
        Ok(())
    }

    // ---- マクロ ---------------------------------------------------------
    fn macro_alloc(&self, id: u16, size: u32) -> HubResult<()> {
        let mut args = id.to_be_bytes().to_vec();
        args.extend_from_slice(&size.to_be_bytes());
        self.req(2, 6, 4, 1, &args)?;
        Ok(())
    }
    fn macro_delete(&self, id: u16) -> HubResult<()> {
        self.req(2, 2, 4, 2, &id.to_be_bytes())?;
        Ok(())
    }
    fn macro_write(&self, id: u16, data: &[u8]) -> HubResult<()> {
        self.macro_alloc(id, data.len() as u32)?;
        for (off, chunk) in data.chunks(51).enumerate() {
            let mut r = [0u8; 64];
            r[2] = 2;
            r[3] = 7 + chunk.len() as u8;
            r[4] = 4;
            r[5] = 3;
            r[6..8].copy_from_slice(&id.to_be_bytes());
            r[8..12].copy_from_slice(&((off * 51) as u32).to_be_bytes());
            r[12] = chunk.len() as u8;
            r[13..13 + chunk.len()].copy_from_slice(chunk);
            self.cmd(&r)?;
        }
        Ok(())
    }
    fn macro_size(&self, id: u16) -> HubResult<u32> {
        let l = self.req(2, 6, 4, 129, &id.to_be_bytes())?;
        // 応答は [6]=cmd echo + [7,8]=id echo → size u32BE は L[9..13]。保険で L[8..12] も見る。
        let s = u32be(&l, 9);
        Ok(if s > 0x10000 { u32be(&l, 8) } else { s })
    }
    fn macro_read(&self, id: u16, size: u32) -> HubResult<Vec<u8>> {
        let mut out = Vec::with_capacity(size as usize);
        while out.len() < size as usize {
            let want = (size as usize - out.len()).min(51) as u8;
            let mut args = id.to_be_bytes().to_vec();
            args.extend_from_slice(&(out.len() as u32).to_be_bytes());
            args.push(want);
            let l = self.req(2, 7 + want, 4, 131, &args)?;
            out.extend_from_slice(&l[13..13 + want as usize]);
        }
        Ok(out)
    }
}

// ==================================================================== ポーリング

pub(crate) fn poll_from_code(code: u8) -> u32 {
    match code {
        // OpenMouse (@openmouse/protocol gwolves) が実機キャプチャで検証:
        // <=1K は周期 ms、>1K は bit4/5/6 の one-hot
        8 => 125,
        4 => 250,
        2 => 500,
        1 => 1000,
        16 => 2000,
        32 => 4000,
        64 => 8000,
        _ => 1000,
    }
}
pub(crate) fn poll_to_code(hz: u32) -> HubResult<u8> {
    Ok(match hz {
        125 => 8,
        250 => 4,
        500 => 2,
        1000 => 1,
        2000 => 16,
        4000 => 32,
        8000 => 64,
        other => return Err(HubError::Invalid(format!("polling {other}Hz"))),
    })
}

// ==================================================================== ボタン変換

/// MouseFn → GW z-id (ButtonCode ctx)。
fn zid_of_mouse(f: MouseFn) -> u8 {
    match f {
        MouseFn::Left => 1,
        MouseFn::Right => 2,
        MouseFn::Middle => 3,
        MouseFn::Back => 4,
        MouseFn::Forward => 5,
        MouseFn::ScrollUp => 16,
        MouseFn::ScrollDown => 17,
        MouseFn::DoubleClick => 1, // X_Click で表現
        _ => 1,
    }
}
fn mouse_of_zid(z: u8) -> Option<MouseFn> {
    Some(match z {
        1 => MouseFn::Left,
        2 => MouseFn::Right,
        3 => MouseFn::Middle,
        4 => MouseFn::Back,
        5 => MouseFn::Forward,
        16 => MouseFn::ScrollUp,
        17 => MouseFn::ScrollDown,
        _ => return None,
    })
}
/// MediaFn → consumer code (u16)。
fn media_code(m: MediaFn) -> u16 {
    match m {
        MediaFn::PlayPause => 0xCD,
        MediaFn::Stop => 0xB7,
        MediaFn::Next => 0xB5,
        MediaFn::Prev => 0xB6,
        MediaFn::VolumeUp => 0xE9,
        MediaFn::VolumeDown => 0xEA,
        MediaFn::Mute => 0xE2,
        MediaFn::Calculator => 0x192,
        MediaFn::MyComputer => 0x194,
        MediaFn::Browser => 0x196,
        MediaFn::Mail => 0x18A,
        MediaFn::BrightnessUp => 0x6F,
        MediaFn::BrightnessDown => 0x70,
        MediaFn::Refresh => 0x7A,
    }
}
fn media_of_code(c: u16) -> Option<MediaFn> {
    Some(match c {
        0xCD => MediaFn::PlayPause,
        0xB7 => MediaFn::Stop,
        0xB5 => MediaFn::Next,
        0xB6 => MediaFn::Prev,
        0xE9 => MediaFn::VolumeUp,
        0xEA => MediaFn::VolumeDown,
        0xE2 => MediaFn::Mute,
        0x192 => MediaFn::Calculator,
        0x194 => MediaFn::MyComputer,
        0x196 => MediaFn::Browser,
        0x18A => MediaFn::Mail,
        _ => return None,
    })
}

fn macro_run_byte(run: MacroRun) -> (u8, u8) {
    // (ie type, times byte): Times→16/n, WhileHeld→17/254, Toggle→18/253。
    match run {
        MacroRun::Times(n) => (16, n.min(252) as u8),
        MacroRun::WhileHeld => (17, 254),
        MacroRun::Toggle => (18, 253),
    }
}

/// GW (XVI=1) エンコード: (ie_type, ctx)。
fn encode_gw(action: &ButtonAction) -> HubResult<(u8, Vec<u8>)> {
    use ButtonAction::*;
    Ok(match action {
        Disabled => (0, vec![]),
        Mouse(MouseFn::DoubleClick) => (2, vec![1, 1, 2, 0, 100]),
        Mouse(f) => (1, vec![zid_of_mouse(*f)]),
        Dpi(DpiFn::Cycle) => (7, vec![6]),
        Dpi(DpiFn::Up) => (7, vec![1]),
        Dpi(DpiFn::Down) => (7, vec![2]),
        Dpi(DpiFn::Shift(d)) => {
            let b = (*d as u16).to_be_bytes();
            (7, vec![5, b[0], b[1], b[0], b[1]])
        }
        Fire {
            button,
            count,
            interval_ms,
        } => (
            2,
            vec![
                1,
                zid_of_mouse(*button),
                *count,
                0,
                (*interval_ms).min(255) as u8,
            ],
        ),
        Key(k) => (4, vec![k.mods, k.usage as u8]),
        Media(m) => (5, media_code(*m).to_be_bytes().to_vec()),
        Macro { slot, run } => {
            let (ty, times) = macro_run_byte(*run);
            (ty, vec![0, slot + 1, times])
        }
        PollingCycle => (13, vec![3]),
        ProfileCycle => (8, vec![3]),
        ScrollModeToggle => return Err(HubError::Unsupported("scroll-mode toggle (GW)")),
        Unsupported { .. } => return Err(HubError::Unsupported("raw action")),
    })
}

/// GW (ie) デコード。
fn decode_gw(ty: u8, ctx: &[u8]) -> ButtonAction {
    use ButtonAction::*;
    let c = |i: usize| ctx.get(i).copied().unwrap_or(0);
    match ty {
        1 => mouse_of_zid(c(0)).map(Mouse).unwrap_or(Disabled),
        2 => Fire {
            button: mouse_of_zid(c(1)).unwrap_or(MouseFn::Left),
            count: c(2),
            interval_ms: c(4) as u16,
        },
        4 => Key(KeyStroke {
            mods: c(0),
            usage: c(1) as u16,
        }),
        5 => media_of_code(((c(0) as u16) << 8) | c(1) as u16)
            .map(Media)
            .unwrap_or(Disabled),
        7 => match c(0) {
            1 => Dpi(DpiFn::Up),
            2 => Dpi(DpiFn::Down),
            5 => Dpi(DpiFn::Shift(((c(1) as u32) << 8) | c(2) as u32)),
            _ => Dpi(DpiFn::Cycle),
        },
        8 => ProfileCycle,
        13 => PollingCycle,
        16..=18 => {
            let slot = c(1).max(1) - 1;
            let run = match ty {
                16 => MacroRun::Times(c(2).max(1) as u16),
                17 => MacroRun::WhileHeld,
                _ => MacroRun::Toggle,
            };
            Macro { slot, run }
        }
        _ => Disabled,
    }
}

/// JM (XVI=0) エンコード: (Ya_type, ctx)。button slot index は呼び側が [7] に入れる。
fn encode_jm(slot: usize, action: &ButtonAction) -> HubResult<(u8, Vec<u8>)> {
    use ButtonAction::*;
    let mouse_mask = |m: u8| vec![m, 0];
    Ok(match action {
        Disabled => (0, vec![]),
        Mouse(MouseFn::ScrollUp) => (11, vec![1, 0]),
        Mouse(MouseFn::ScrollDown) => (11, vec![2, 0]),
        Mouse(MouseFn::DoubleClick) => (4, vec![100, 2]),
        Mouse(f) => (
            1,
            mouse_mask(match f {
                MouseFn::Left => 1,
                MouseFn::Right => 2,
                MouseFn::Middle => 4,
                MouseFn::Back => 8,
                MouseFn::Forward => 16,
                _ => 1,
            }),
        ),
        Dpi(DpiFn::Cycle) => (2, vec![1, 0]),
        Dpi(DpiFn::Up) => (2, vec![2, 0]),
        Dpi(DpiFn::Down) => (2, vec![3, 0]),
        Dpi(DpiFn::Shift(d)) => (10, vec![0, ((*d / 50).max(1) - 1) as u8]),
        Fire {
            count, interval_ms, ..
        } => (4, vec![(*interval_ms).min(255) as u8, *count]),
        Key(k) => {
            // ShortcutKey: [count*2, 各3B: (type|0x80 down / |0x40 up), lo, hi]
            let mut events: Vec<[u8; 3]> = vec![];
            for bit in 0..8 {
                if k.mods >> bit & 1 == 1 {
                    events.push([0x80, 1 << bit, 0]); // mod down (type 0)
                }
            }
            events.push([0x81, (k.usage & 0xFF) as u8, (k.usage >> 8) as u8]);
            for bit in 0..8 {
                if k.mods >> bit & 1 == 1 {
                    events.push([0x40, 1 << bit, 0]); // mod up
                }
            }
            events.push([0x41, (k.usage & 0xFF) as u8, (k.usage >> 8) as u8]);
            let mut ctx = vec![events.len() as u8];
            for e in events {
                ctx.extend_from_slice(&e);
            }
            (5, ctx)
        }
        Media(m) => {
            let c = media_code(*m);
            (
                5,
                vec![
                    2,
                    0x82,
                    (c & 0xFF) as u8,
                    (c >> 8) as u8,
                    0x42,
                    (c & 0xFF) as u8,
                    (c >> 8) as u8,
                    0,
                ],
            )
        }
        Macro { slot: mslot, run } => {
            // ctx[0]=対象 (実機検証待ち: JS は btn slot を入れるが macro index と解釈して送る),
            // ctx[1]=cycle (Times=n / WhileHeld=254 / Toggle=253)。
            let (_, times) = macro_run_byte(*run);
            let _ = slot;
            (6, vec![mslot + 1, times])
        }
        PollingCycle => (7, vec![0]),
        ProfileCycle => (8, vec![3]),
        ScrollModeToggle => return Err(HubError::Unsupported("scroll-mode toggle (JM)")),
        Unsupported { .. } => return Err(HubError::Unsupported("raw action")),
    })
}

/// JM (Ya) デコード。
fn decode_jm(ty: u8, ctx: &[u8]) -> ButtonAction {
    use ButtonAction::*;
    let c = |i: usize| ctx.get(i).copied().unwrap_or(0);
    match ty {
        1 => match c(0) {
            1 => Mouse(MouseFn::Left),
            2 => Mouse(MouseFn::Right),
            4 => Mouse(MouseFn::Middle),
            8 => Mouse(MouseFn::Back),
            16 => Mouse(MouseFn::Forward),
            _ => Disabled,
        },
        2 => match c(0) {
            2 => Dpi(DpiFn::Up),
            3 => Dpi(DpiFn::Down),
            _ => Dpi(DpiFn::Cycle),
        },
        4 => {
            if c(0) == 100 && c(1) == 2 {
                Mouse(MouseFn::DoubleClick)
            } else {
                Fire {
                    button: MouseFn::Left,
                    count: c(1),
                    interval_ms: c(0) as u16,
                }
            }
        }
        5 => {
            // メディア: [2, 0x82, lo, hi, 0x42, ...]
            if c(0) == 2 && c(1) == 0x82 {
                media_of_code(c(2) as u16 | ((c(3) as u16) << 8))
                    .map(Media)
                    .unwrap_or(Disabled)
            } else {
                // キー: イベント列から mods+usage を復元
                let n = c(0) as usize;
                let mut mods = 0u8;
                let mut usage = 0u16;
                for i in 0..n {
                    let b = 1 + i * 3;
                    let t = ctx.get(b).copied().unwrap_or(0) & 0x3F;
                    let v = c(b + 1) as u16 | ((c(b + 2) as u16) << 8);
                    if ctx.get(b).copied().unwrap_or(0) & 0x80 != 0 {
                        if t == 0 {
                            mods |= v as u8;
                        } else if t == 1 {
                            usage = v;
                        }
                    }
                }
                Key(KeyStroke { mods, usage })
            }
        }
        6 => {
            let slot = c(0).max(1) - 1;
            let run = match c(1) {
                254 => MacroRun::WhileHeld,
                253 => MacroRun::Toggle,
                n => MacroRun::Times(n.max(1) as u16),
            };
            Macro { slot, run }
        }
        7 => PollingCycle,
        8 => ProfileCycle,
        10 => Dpi(DpiFn::Shift(((c(1) as u32) + 1) * 50)),
        11 => Mouse(if c(0) == 2 {
            MouseFn::ScrollDown
        } else {
            MouseFn::ScrollUp
        }),
        _ => Disabled,
    }
}

// ==================================================================== マクロ変換

/// MacroDef.events → mouse.fit イベントバイト列 (ec() 相当)。
fn encode_macro(def: &MacroDef) -> Vec<u8> {
    let mut out = Vec::new();
    for ev in &def.events {
        match ev.input {
            MacroInput::Key(u) if (0xE0..=0xE7).contains(&u) => {
                let bit = 1u8 << (u - 0xE0);
                out.extend_from_slice(&[if ev.edge == MacroEdge::Down { 9 } else { 10 }, bit]);
            }
            MacroInput::Key(u) => {
                out.extend_from_slice(&[if ev.edge == MacroEdge::Down { 2 } else { 3 }, u as u8]);
            }
            MacroInput::Mouse(f) => {
                let mask = match f {
                    MouseFn::Left => 1,
                    MouseFn::Right => 2,
                    MouseFn::Middle => 4,
                    MouseFn::Back => 8,
                    MouseFn::Forward => 16,
                    MouseFn::ScrollUp => {
                        out.extend_from_slice(&[16, 1]);
                        continue;
                    }
                    MouseFn::ScrollDown => {
                        out.extend_from_slice(&[16, 255]);
                        continue;
                    }
                    _ => 0,
                };
                out.extend_from_slice(&[1, if ev.edge == MacroEdge::Down { mask } else { 0 }]);
            }
        }
        if ev.delay_ms > 0 {
            push_delay(&mut out, ev.delay_ms);
        }
    }
    out
}

fn push_delay(out: &mut Vec<u8>, ms: u32) {
    if ms <= 0xFF {
        out.extend_from_slice(&[32, ms as u8]);
    } else if ms <= 0xFFFF {
        out.extend_from_slice(&[33, (ms >> 8) as u8, ms as u8]);
    } else if ms <= 0xFF_FFFF {
        out.extend_from_slice(&[34, (ms >> 16) as u8, (ms >> 8) as u8, ms as u8]);
    } else {
        out.push(35);
        out.extend_from_slice(&ms.to_be_bytes());
    }
}

/// バイト列 → MacroDef.events (decode)。mode/repeat は束縛側の情報なので既定値。
fn decode_macro(data: &[u8]) -> Vec<MacroEvent> {
    let mut evs = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let ty = data[i];
        match ty {
            1 | 2 | 3 | 9 | 10 | 16 => {
                if i + 1 >= data.len() {
                    break;
                }
                let v = data[i + 1];
                let (input, edge) = match ty {
                    2 | 3 => (
                        MacroInput::Key(v as u16),
                        if ty == 2 {
                            MacroEdge::Down
                        } else {
                            MacroEdge::Up
                        },
                    ),
                    9 | 10 => {
                        // modbit → usage 0xE0+bit index
                        let bit = v.trailing_zeros().min(7) as u16;
                        (
                            MacroInput::Key(0xE0 + bit),
                            if ty == 9 {
                                MacroEdge::Down
                            } else {
                                MacroEdge::Up
                            },
                        )
                    }
                    16 => (
                        MacroInput::Mouse(if v == 255 {
                            MouseFn::ScrollDown
                        } else {
                            MouseFn::ScrollUp
                        }),
                        MacroEdge::Down,
                    ),
                    _ => {
                        // ty==1: マウスボタン。mask→down, 0→up。
                        let f = match v {
                            1 => MouseFn::Left,
                            2 => MouseFn::Right,
                            4 => MouseFn::Middle,
                            8 => MouseFn::Back,
                            16 => MouseFn::Forward,
                            _ => MouseFn::Left,
                        };
                        (
                            MacroInput::Mouse(f),
                            if v == 0 {
                                MacroEdge::Up
                            } else {
                                MacroEdge::Down
                            },
                        )
                    }
                };
                evs.push(MacroEvent {
                    input,
                    edge,
                    delay_ms: 0,
                });
                i += 2;
            }
            32..=35 => {
                let nb = (ty - 31) as usize; // 32→1B .. 35→4B
                if i + 1 + nb > data.len() {
                    break;
                }
                let mut ms = 0u32;
                for j in 0..nb {
                    ms = (ms << 8) | data[i + 1 + j] as u32;
                }
                if let Some(last) = evs.last_mut() {
                    last.delay_ms = ms;
                }
                i += 1 + nb;
            }
            _ => break,
        }
    }
    evs
}

// ==================================================================== caps

fn opts(v: &[(i64, &str)]) -> Vec<ChoiceOpt> {
    v.iter().map(|(a, b)| ChoiceOpt::new(*a, *b)).collect()
}

fn build_caps(info: &DeviceInfo, m: &MfModel) -> Capabilities {
    use section::*;
    let wireless = info.transport == Transport::Wireless24G;
    let poll_opts: Vec<ChoiceOpt> = m
        .polling
        .iter()
        .map(|h| {
            ChoiceOpt::new(
                *h as i64,
                match h {
                    1000 => "1K".to_string(),
                    2000 => "2K".to_string(),
                    4000 => "4K".to_string(),
                    8000 => "8K".to_string(),
                    _ => h.to_string(),
                },
            )
        })
        .collect();
    let mut settings = vec![SettingSpec::choice(
        "polling_wired",
        PERFORMANCE,
        "ポーリングレート (有線)",
        poll_opts,
    )
    .help("マウスがPCへ位置を報告する頻度 (Hz)")];
    if wireless {
        settings.push(
            SettingSpec::choice(
                "polling_wireless",
                PERFORMANCE,
                "ポーリングレート (無線)",
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
            .help("使用中のレシーバー (1K / 4K / 8K) で選べる値が変わります"),
        );
    }
    settings.extend([
        SettingSpec::toggle("competitive", PERFORMANCE, "競技モード")
            .help("センサーを常時最高性能で動かします (Tracking Mode)"),
        SettingSpec::choice("lod", SENSOR, "リフトオフ距離", opts(m.lod_choices))
            .help("マウスを持ち上げたとき反応しなくなる高さ"),
        SettingSpec::toggle("motion_sync", SENSOR, "モーション同期")
            .help("ポインタとマウスの動きを同期させ、軌跡を滑らかにします"),
        SettingSpec::toggle("angle_snap", SENSOR, "直線補正").help("移動のブレを直線に補正します"),
        SettingSpec::toggle("ripple", SENSOR, "リップル制御").help("高DPI時の細かい揺れを抑えます"),
        SettingSpec::slider("sensor_angle", SENSOR, "センサー角度", -30., 30., 1., "°")
            .help("持ち方に合わせて座標軸を回転させます"),
        SettingSpec::toggle("dpi_xy", SENSOR, "DPI X/Y 個別設定")
            .help("横方向と縦方向で別々のDPIにします"),
        SettingSpec::slider(
            "debounce_press",
            CLICK,
            "クリックデバウンス (押下)",
            0.,
            50.,
            1.,
            "ms",
        )
        .help("短くすると反応が速く、長くすると誤クリックを防ぎます")
        .confirm("0ms にするとダブルクリックが発生する場合があります。続けますか?"),
        SettingSpec::slider(
            "debounce_release",
            CLICK,
            "クリックデバウンス (解放)",
            0.,
            50.,
            1.,
            "ms",
        ),
        SettingSpec::slider(
            "wheel_debounce",
            CLICK,
            "ホイールデバウンス",
            0.,
            245.,
            5.,
            "ms",
        ),
    ]);
    if m.spdt {
        settings.extend([
            SettingSpec::toggle("spdt_left", CLICK, "Left SPDT")
                .help("光学/機械スイッチの SPDT 方式でチャタリングを防ぎます"),
            SettingSpec::toggle("spdt_right", CLICK, "Right SPDT"),
        ]);
    }
    if m.liftoff_debounce {
        settings.push(
            SettingSpec::slider(
                "liftoff_debounce",
                CLICK,
                "持ち上げデバウンス",
                0.,
                30.,
                1.,
                "ms",
            )
            .help("持ち上げた瞬間のクリックを無視する時間"),
        );
    }
    if wireless {
        settings.push(
            SettingSpec::choice(
                "sleep_min",
                POWER,
                "スリープまでの時間",
                opts(&[
                    (1, "1分"),
                    (2, "2分"),
                    (3, "3分"),
                    (5, "5分"),
                    (10, "10分"),
                    (15, "15分"),
                    (30, "30分"),
                ]),
            )
            .help("この時間動かさないとスリープします"),
        );
    }
    settings.push(
        SettingSpec::toggle("dpi_indicator", POWER, "DPI インジケーター")
            .help("DPI 切替時に LED で知らせます"),
    );

    Capabilities {
        settings,
        actions: vec![],
        profiles: None, // open() 後に caps へ入れるため profile_count を別途反映
        mouse: Some(MouseCaps {
            dpi_min: 50,
            commit_dpi_on_release: false,
            dpi_max: m.dpi_max,
            dpi_step: m.dpi_step,
            min_stages: 1,
            max_stages: m.max_stages,
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
                ButtonActionKind::Dpi,
                ButtonActionKind::Fire,
                ButtonActionKind::Key,
                ButtonActionKind::Media,
                ButtonActionKind::Macro,
                ButtonActionKind::PollingCycle,
                ButtonActionKind::ProfileCycle,
                ButtonActionKind::Disabled,
            ],
            button_options: None,
            require_left_click: true,
            live_buttons: vec![],
            adaptive_dpi: None,
            macros: Some(MacroCaps {
                slots: 3,
                max_events: 70,
                max_delay_ms: 0xFF_FFFF,
                modes: opts(&[
                    (0, "指定回数実行"),
                    (1, "離すまで繰り返す"),
                    (2, "もう一度押すまで繰り返す"),
                ]),
                repeat_modes: vec![0],
                names: false,
                mouse_events: true,
            }),
        }),
        keyboard: None,
        lighting: vec![],
        battery: wireless,
        peq: None,
        firmware_feed: None,
    }
}

// ==================================================================== DeviceDriver

impl DeviceDriver for MousefitDriver {
    fn info(&self) -> DeviceInfo {
        let mut i = self.info.clone();
        i.firmware = self
            .firmware
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        i.dongle_firmware = self
            .dongle_firmware
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Ok(b) = self.battery() {
            i.battery = Some(b);
        }
        i
    }

    fn caps(&self) -> Capabilities {
        let mut c = self.caps.clone();
        // プロファイル数は機種/FW 依存 → 読めたら宣言 (読めなくても caps は返す)。
        let n = self.profile_count();
        if n > 1 {
            c.profiles = Some(ProfileCaps {
                count: n,
                renamable: false,
            });
        }
        c
    }

    fn read(&self) -> HubResult<DeviceState> {
        let prof = self.profile()?;
        let mut settings: BTreeMap<String, SettingValue> = BTreeMap::new();
        macro_rules! get {
            ($id:literal, $r:expr $(,)?) => {
                if let Ok(v) = $r {
                    settings.insert($id.to_string(), v);
                }
            };
        }
        get!(
            "polling_wired",
            self.polling(prof).map(|h| SettingValue::Choice(h as i64))
        );
        if self.caps.setting("polling_wireless").is_some() {
            get!(
                "polling_wireless",
                self.polling(prof).map(|h| SettingValue::Choice(h as i64))
            );
        }
        get!(
            "competitive",
            self.bool_get(prof, 2, 1, 147).map(SettingValue::Bool)
        );
        get!(
            "lod",
            self.lod(prof).map(|w| {
                let tenth = if self.model.lod_is_index {
                    // 1-based index → choice の i64 値
                    self.model
                        .lod_choices
                        .get(w.max(1) as usize - 1)
                        .map(|(v, _)| *v)
                        .unwrap_or(7)
                } else {
                    match w {
                        0x87 | 7 => 7,
                        1 => 10,
                        2 => 20,
                        v => v as i64 * 10,
                    }
                };
                SettingValue::Choice(tenth)
            }),
        );
        get!(
            "motion_sync",
            self.bool_get(prof, 2, 1, 137).map(SettingValue::Bool)
        );
        get!(
            "angle_snap",
            self.bool_get(prof, 2, 1, 132).map(SettingValue::Bool)
        );
        get!(
            "ripple",
            self.bool_get(prof, 2, 1, 138).map(SettingValue::Bool)
        );
        get!(
            "sensor_angle",
            self.read8(prof, 2, 1, 148).map(|v| {
                let v = if v > 128 { v as i16 - 256 } else { v as i16 };
                SettingValue::Num(v as f32)
            }),
        );
        get!(
            "dpi_xy",
            self.bool_get(prof, 2, 1, 141).map(SettingValue::Bool)
        );
        if let Ok(d) = self.debounce(prof) {
            settings.insert("debounce_press".into(), SettingValue::Num(d[0] as f32));
            settings.insert("debounce_release".into(), SettingValue::Num(d[1] as f32));
        }
        if let Ok((_rate, deb)) = self.scroll_scan(prof) {
            settings.insert("wheel_debounce".into(), SettingValue::Num(deb as f32));
        }
        if self.model.spdt {
            if let Ok((l, r)) = self.spdt(prof) {
                settings.insert("spdt_left".into(), SettingValue::Bool(l));
                settings.insert("spdt_right".into(), SettingValue::Bool(r));
            }
        }
        if self.model.liftoff_debounce {
            get!(
                "liftoff_debounce",
                self.read8(prof, 2, 0, 155)
                    .map(|v| SettingValue::Num(v as f32)),
            );
        }
        if self.caps.setting("sleep_min").is_some() {
            get!(
                "sleep_min",
                self.sleep_min(prof).map(|m| SettingValue::Choice(m as i64)),
            );
        }
        get!(
            "dpi_indicator",
            self.bool_get(prof, 2, 2, 132).map(SettingValue::Bool)
        );

        // DPI ステージ
        let (n, xy) = self.dpi_stages(prof)?;
        let colors = self.dpi_colors(prof).unwrap_or_default();
        let active = self
            .active_stage(prof)
            .unwrap_or(0)
            .min(n.saturating_sub(1));
        let stages: Vec<DpiStage> = xy
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| DpiStage {
                x,
                y: Some(y),
                color: colors.get(i).copied().unwrap_or(Rgb::hex(0xffffff)),
                enabled: true,
            })
            .collect();

        // ボタン (左/右/中/戻/進 = zid 1..5 / JM slot 0..4)
        let mut buttons = Vec::with_capacity(5);
        for slot in 0..5usize {
            let id = if self.model.xvi {
                slot as u8 + 1
            } else {
                slot as u8
            };
            let action = self
                .get_button(prof, id)
                .map(|(ty, ctx)| {
                    if self.model.xvi {
                        decode_gw(ty, &ctx)
                    } else {
                        decode_jm(ty, &ctx)
                    }
                })
                .unwrap_or(ButtonAction::Disabled);
            buttons.push(action);
        }

        // マクロ (slot 0..2 → device id 1..3)
        let mut macros = Vec::with_capacity(3);
        for slot in 0..3u16 {
            let def = self
                .macro_size(slot + 1)
                .ok()
                .filter(|&s| s > 0 && s <= 4096)
                .and_then(|s| self.macro_read(slot + 1, s).ok())
                .map(|data| MacroDef {
                    name: format!("マクロ {}", slot + 1),
                    events: decode_macro(&data),
                    mode: 0,
                    repeat: 1,
                });
            macros.push(def);
        }

        Ok(DeviceState {
            profile: ProfileState {
                active: prof,
                names: vec![],
            },
            settings,
            mouse: Some(MouseState {
                stages,
                active_stage: active,
                buttons,
                macros,
                adaptive: None,
            }),
            keyboard: None,
            lighting: BTreeMap::new(),
            peq: None,
        })
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        let prof = self.profile()?;
        match change {
            Change::Setting { id, value } => self.apply_setting(prof, id, value),
            Change::Profile(p) => self.set_profile(*p),
            Change::DpiStages { stages, active } => {
                let n = stages.len().clamp(1, self.model.max_stages);
                let xy: Vec<(u32, u32)> = stages[..n]
                    .iter()
                    .map(|s| (s.x, s.y.unwrap_or(s.x)))
                    .collect();
                // JS と同じ順序: 先に active を clamp して書き、続けてステージと色。
                self.set_active_stage(prof, (*active).min(n - 1))?;
                self.write_dpi_stages(prof, &xy)?;
                let colors: Vec<Rgb> = stages[..n].iter().map(|s| s.color).collect();
                self.set_dpi_colors(prof, &colors)
            }
            Change::ActiveStage(a) => self.set_active_stage(prof, *a),
            Change::Button { slot, action } => {
                if *slot >= 5 {
                    return Err(HubError::Invalid("button slot".into()));
                }
                let (id, ty, ctx) = if self.model.xvi {
                    let (t, c) = encode_gw(action)?;
                    (*slot as u8 + 1, t, c)
                } else {
                    let (t, c) = encode_jm(*slot, action)?;
                    (*slot as u8, t, c)
                };
                self.set_button(prof, id, ty, &ctx)
            }
            Change::Macro { slot, def } => {
                if *slot >= 3 {
                    return Err(HubError::Invalid("macro slot".into()));
                }
                let id = *slot as u16 + 1;
                match def {
                    None => self.macro_delete(id),
                    Some(d) => {
                        let data = encode_macro(d);
                        if data.is_empty() {
                            self.macro_delete(id)
                        } else {
                            self.macro_write(id, &data)
                        }
                    }
                }
            }
            Change::Lighting { .. } => Err(HubError::Unsupported("lighting")),
            _ => Err(HubError::Unsupported("change")),
        }
    }

    fn battery(&self) -> HubResult<Battery> {
        let l = self.req(2, 2, 0, 131, &[])?;
        if l[6] != 131 {
            return Err(HubError::Protocol("battery ack".into()));
        }
        Ok(Battery {
            percent: Some(l[7].min(100)),
            charging: false,
        })
    }
}

impl MousefitDriver {
    fn apply_setting(&self, prof: u8, id: &str, v: &SettingValue) -> HubResult<()> {
        match id {
            "polling_wired" | "polling_wireless" => self.set_polling(prof, v.as_choice() as u32),
            "competitive" => self.bool_set(prof, 2, 1, 19, v.as_bool()),
            "lod" => {
                let tenth = v.as_choice();
                let wire = if self.model.lod_is_index {
                    (self
                        .model
                        .lod_choices
                        .iter()
                        .position(|(c, _)| *c == tenth)
                        .ok_or_else(|| HubError::Invalid("lod".into()))?
                        + 1) as u8
                } else {
                    // 実値: 0.7 → (7*10|128)=0x87, >=1 → mm 値
                    if tenth >= 10 {
                        (tenth / 10) as u8
                    } else {
                        (tenth as u8) | 0x80
                    }
                };
                self.set_lod(prof, wire)
            }
            "motion_sync" => self.bool_set(prof, 2, 1, 9, v.as_bool()),
            "angle_snap" => self.bool_set(prof, 2, 1, 4, v.as_bool()),
            "ripple" => self.bool_set(prof, 2, 1, 10, v.as_bool()),
            "sensor_angle" => {
                let d = v.as_num().round() as i16;
                let enc = if d > 0 {
                    d as u8
                } else {
                    (255 - d.unsigned_abs() + 1) as u8
                };
                self.write1(prof, 2, 1, 20, enc)
            }
            "dpi_xy" => self.bool_set(prof, 2, 1, 13, v.as_bool()),
            "debounce_press" | "debounce_release" => {
                let mut d = self.debounce(prof)?;
                if id == "debounce_press" {
                    d[0] = v.as_num().clamp(0., 255.) as u8;
                } else {
                    d[1] = v.as_num().clamp(0., 255.) as u8;
                }
                self.set_debounce(prof, &d)
            }
            "wheel_debounce" => {
                let (rate, _) = self.scroll_scan(prof)?;
                self.set_scroll_scan(prof, rate, v.as_num().clamp(0., 245.) as u8)
            }
            "spdt_left" | "spdt_right" => {
                let (l, r) = self.spdt(prof)?;
                let (l, r) = if id == "spdt_left" {
                    (v.as_bool(), r)
                } else {
                    (l, v.as_bool())
                };
                self.set_spdt(prof, l, r)
            }
            "liftoff_debounce" => self.write1(prof, 2, 0, 27, v.as_num().clamp(0., 255.) as u8),
            "sleep_min" => self.set_sleep_min(prof, v.as_choice().clamp(0, 0xFFFF) as u16),
            "dpi_indicator" => self.bool_set(prof, 2, 2, 4, v.as_bool()),
            other => Err(HubError::Invalid(format!("unknown setting {other}"))),
        }
    }
}

// ==================================================================== テスト

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(payload: &[u8; 64]) -> [u8; 65] {
        let mut b = [0u8; 65];
        b[1..].copy_from_slice(payload);
        b
    }

    #[test]
    fn logical_hidindex() {
        // hi=1: payload[0]=0xA1 → L[1]=buf[1]
        let mut p = [0u8; 64];
        p[0] = 0xA1;
        p[5] = 0x81;
        p[7] = 42;
        let l = logical(&resp(&p));
        assert_eq!(l[1], 0xA1);
        assert_eq!(l[6], 0x81);
        assert_eq!(l[8], 42);
        // hi=0: payload[0]<0xA0 → L[i]=buf[1+i]
        let mut p = [0u8; 64];
        p[0] = 0x10;
        p[1] = 0xA1;
        p[8] = 77;
        let l = logical(&resp(&p));
        assert_eq!(l[1], 0xA1);
        assert_eq!(l[8], 77);
    }

    #[test]
    fn polling_map() {
        for (hz, code) in [
            (125u32, 8u8),
            (250, 4),
            (500, 2),
            (1000, 1),
            (2000, 16),
            (4000, 32),
            (8000, 64),
        ] {
            assert_eq!(poll_to_code(hz).unwrap(), code);
            assert_eq!(poll_from_code(code), hz);
        }
    }

    #[test]
    fn dpi_stage_records() {
        // L[8]=count, L[9+4i..]=x hi,lo,y hi,lo
        let mut l = [0u8; 64];
        l[8] = 2;
        l[9..13].copy_from_slice(&[0x06, 0x40, 0x06, 0x40]); // 1600,1600
        l[13..17].copy_from_slice(&[0x0C, 0x80, 0x03, 0x20]); // 3200,800
        assert_eq!(u16be(&l, 9), 1600);
        assert_eq!(u16be(&l, 11), 1600);
        assert_eq!(u16be(&l, 13), 3200);
        assert_eq!(u16be(&l, 15), 800);
    }

    #[test]
    fn gw_button_roundtrip() {
        let (ty, ctx) = encode_gw(&ButtonAction::Key(KeyStroke {
            mods: mods::LCTRL,
            usage: 0x06, // C
        }))
        .unwrap();
        assert_eq!(ty, 4);
        assert_eq!(ctx, vec![1, 6]);
        match decode_gw(ty, &ctx) {
            ButtonAction::Key(k) => {
                assert_eq!(k.mods, 1);
                assert_eq!(k.usage, 6);
            }
            other => panic!("{other:?}"),
        }
        // DPI shift
        let (ty, ctx) = encode_gw(&ButtonAction::Dpi(DpiFn::Shift(400))).unwrap();
        assert_eq!(ty, 7);
        assert_eq!(ctx, vec![5, 1, 0x90, 1, 0x90]);
        assert_eq!(decode_gw(ty, &ctx), ButtonAction::Dpi(DpiFn::Shift(400)));
        // マクロ
        let (ty, ctx) = encode_gw(&ButtonAction::Macro {
            slot: 2,
            run: MacroRun::WhileHeld,
        })
        .unwrap();
        assert_eq!(ty, 17);
        assert_eq!(ctx, vec![0, 3, 254]);
        assert_eq!(
            decode_gw(ty, &ctx),
            ButtonAction::Macro {
                slot: 2,
                run: MacroRun::WhileHeld
            }
        );
    }

    #[test]
    fn jm_button_roundtrip() {
        let (ty, ctx) = encode_jm(3, &ButtonAction::Mouse(MouseFn::Back)).unwrap();
        assert_eq!(ty, 1);
        assert_eq!(ctx, vec![8, 0]);
        assert_eq!(decode_jm(ty, &ctx), ButtonAction::Mouse(MouseFn::Back));
        // media
        let (ty, ctx) = encode_jm(0, &ButtonAction::Media(MediaFn::PlayPause)).unwrap();
        assert_eq!(ty, 5);
        assert_eq!(ctx, vec![2, 0x82, 0xCD, 0, 0x42, 0xCD, 0, 0]);
        assert_eq!(decode_jm(ty, &ctx), ButtonAction::Media(MediaFn::PlayPause));
        // key combo
        let (ty, ctx) = encode_jm(
            1,
            &ButtonAction::Key(KeyStroke {
                mods: mods::LCTRL | mods::LSHIFT,
                usage: 0x04,
            }),
        )
        .unwrap();
        assert_eq!(ty, 5);
        // [n=4, mod down×2, key down, mod up×2, key up] = 13B
        assert_eq!(ctx[0], 6);
        match decode_jm(ty, &ctx) {
            ButtonAction::Key(k) => {
                assert_eq!(k.mods, 0b11);
                assert_eq!(k.usage, 4);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn macro_encode_decode() {
        let def = MacroDef {
            name: "t".into(),
            mode: 0,
            repeat: 1,
            events: vec![
                MacroEvent {
                    input: MacroInput::Key(0xE0), // LCtrl
                    edge: MacroEdge::Down,
                    delay_ms: 0,
                },
                MacroEvent {
                    input: MacroInput::Key(0x04), // A
                    edge: MacroEdge::Down,
                    delay_ms: 50,
                },
                MacroEvent {
                    input: MacroInput::Key(0x04),
                    edge: MacroEdge::Up,
                    delay_ms: 300,
                },
                MacroEvent {
                    input: MacroInput::Mouse(MouseFn::Left),
                    edge: MacroEdge::Down,
                    delay_ms: 0,
                },
                MacroEvent {
                    input: MacroInput::Mouse(MouseFn::Left),
                    edge: MacroEdge::Up,
                    delay_ms: 0,
                },
            ],
        };
        let data = encode_macro(&def);
        assert_eq!(data, vec![9, 1, 2, 4, 32, 50, 3, 4, 33, 1, 44, 1, 1, 1, 0]);
        let back = decode_macro(&data);
        assert_eq!(back, def.events);
    }

    #[test]
    fn macro_delay_sizes() {
        let mut v = Vec::new();
        push_delay(&mut v, 100);
        assert_eq!(v, vec![32, 100]);
        v.clear();
        push_delay(&mut v, 300);
        assert_eq!(v, vec![33, 1, 44]);
        v.clear();
        push_delay(&mut v, 0x1_0000 + 5);
        assert_eq!(v, vec![34, 1, 0, 5]);
    }

    #[test]
    fn lod_wire() {
        // JM index: choice値 7 → index 1 → wire 1
        assert_eq!(HTS_PLUS_PRO.lod_choices[0].0, 7);
        // GW 実値: 0.7mm → 0x87
        assert_eq!((7u8) | 0x80, 0x87);
    }
}

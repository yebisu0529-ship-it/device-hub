//! デモ用バックエンド — 実機なしで UI のすべての画面を動かす。
//!
//! **ドライバ実装者への見本も兼ねる**: `mouse_caps` / `keyboard_caps` は
//! mouse.fit (G-Wolves HTS Plus Pro) と hub.xsyd.top (SU66 Max) の Web 版にある
//! 機能をそのまま宣言している。実ドライバはこれと同じ形の caps を返せばよい
//! (値の範囲・選択肢は実機/JS バンドルで確認して差し替える)。

use crate::backend::*;
use crate::keys;
use crate::model::*;
use crate::registry;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct DemoHub {
    drivers: Vec<Arc<DemoDriver>>,
    unsupported: DeviceInfo,
}

impl DemoHub {
    pub fn new() -> Self {
        let mut drivers = vec![
            Arc::new(DemoDriver::mouse(
                "demo:hts-plus-pro",
                "HTS Plus Pro",
                "hts-plus-pro",
                0x5219,
                Transport::Wireless24G,
                Some(87),
                false,
            )),
            Arc::new(DemoDriver::keyboard()),
            Arc::new(DemoDriver::mouse(
                "demo:htx-ultra",
                "HTX Ultra",
                "htx-ultra",
                0x5608,
                Transport::Wired,
                None,
                false,
            )),
            Arc::new(DemoDriver::dac()),
            Arc::new(DemoDriver::lamzu()),
        ];
        drivers.shrink_to_fit();
        Self {
            drivers,
            unsupported: DeviceInfo {
                id: DeviceId("demo:atk-f1".into()),
                kind: DeviceKind::Mouse,
                name: "ATK F1 Ultimate".into(),
                vendor: "ATK".into(),
                model: "atk-f1-ultimate".into(),
                vid: 0x373B,
                pid: 0x1000,
                transport: Transport::Wireless24G,
                firmware: None,
                dongle_firmware: None,
                serial: None,
                driver: DriverKind::Unsupported,
                battery: None,
                supported: false,
                asleep: false,
                board_id: None,
            },
        }
    }
}

impl Default for DemoHub {
    fn default() -> Self {
        Self::new()
    }
}

/// watch で受け取った通知先 (デモは 1 プロセス 1 ハブなので static で足りる)。
type Sink = Box<dyn Fn(HubEvent) + Send + Sync>;
static SINK: Mutex<Option<Sink>> = Mutex::new(None);

fn emit(ev: HubEvent) {
    if let Some(f) = SINK.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        f(ev);
    }
}

impl HubBackend for DemoHub {
    fn watch(&self, cb: Box<dyn Fn(HubEvent) + Send + Sync>) {
        *SINK.lock().unwrap_or_else(|p| p.into_inner()) = Some(cb);
    }

    fn scan(&self) -> HubResult<Vec<DeviceInfo>> {
        let mut v: Vec<DeviceInfo> = self.drivers.iter().map(|d| d.info()).collect();
        v.push(self.unsupported.clone());
        Ok(v)
    }

    fn driver(&self, id: &DeviceId) -> HubResult<Arc<dyn DeviceDriver>> {
        if *id == self.unsupported.id {
            return Err(HubError::NotImplemented("ATK F1 Ultimate のプロトコル"));
        }
        self.drivers
            .iter()
            .find(|d| d.info.id == *id)
            .map(|d| d.clone() as Arc<dyn DeviceDriver>)
            .ok_or_else(|| HubError::NotFound(id.clone()))
    }

    fn is_demo(&self) -> bool {
        true
    }
}

// ====================================================================== ドライバ

pub struct DemoDriver {
    info: DeviceInfo,
    caps: Capabilities,
    state: Mutex<DeviceState>,
    /// 構成 (オンボードプロファイル) ごとの退避状態 — 実機と同じく
    /// 構成を切り替えると設定の中身もまるごと入れ替わる。
    saved: Mutex<std::collections::BTreeMap<u8, DeviceState>>,
    /// 一度も触っていない構成を開いたときの初期値。
    factory: DeviceState,
    born: Instant,
}

impl DemoDriver {
    fn mouse(
        id: &str,
        name: &str,
        model: &str,
        pid: u16,
        transport: Transport,
        battery: Option<u8>,
        magnetic: bool,
    ) -> Self {
        let caps = mouse_caps(transport == Transport::Wireless24G, magnetic);
        let state = mouse_state(&caps);
        Self {
            info: DeviceInfo {
                id: DeviceId(id.into()),
                kind: DeviceKind::Mouse,
                name: name.into(),
                vendor: "G-Wolves".into(),
                model: model.into(),
                vid: 0x33E4,
                pid,
                transport,
                firmware: Some("2.05".into()),
                dongle_firmware: (transport == Transport::Wireless24G).then(|| "1.12".into()),
                serial: None,
                driver: DriverKind::Mousefit,
                battery: battery.map(|p| Battery {
                    percent: Some(p),
                    charging: false,
                }),
                supported: true,
                asleep: false,
                board_id: None,
            },
            caps,
            state: Mutex::new(state.clone()),
            saved: Mutex::new(Default::default()),
            factory: state,
            born: Instant::now(),
        }
    }

    fn keyboard() -> Self {
        let caps = keyboard_caps();
        let state = keyboard_state(&caps);
        Self {
            info: DeviceInfo {
                id: DeviceId("demo:su66max".into()),
                kind: DeviceKind::Keyboard,
                name: "SU66 Max".into(),
                vendor: "XSYD".into(),
                model: "su66max".into(),
                vid: 0x1CA6,
                pid: 0x3009,
                transport: Transport::Wired,
                firmware: Some("1.4.2".into()),
                dongle_firmware: None,
                serial: Some("49142".into()),
                driver: DriverKind::Xsyd,
                battery: None,
                supported: true,
                asleep: false,
                board_id: None,
            },
            caps,
            state: Mutex::new(state.clone()),
            saved: Mutex::new(Default::default()),
            factory: state,
            born: Instant::now(),
        }
    }

    /// peq.szwalkplay.com (WalkPlay TP55) 相当の USB DAC。
    /// LAMZU MAYA X V2 (8K レシーバー経由) — 公式 Aurora と同じ caps (`crate::lamzu`)。
    fn lamzu() -> Self {
        let model = &crate::lamzu::MODELS[0];
        let caps = crate::lamzu::caps(model, Transport::Wireless24G, 4);
        let state = lamzu_state(&caps);
        Self {
            info: DeviceInfo {
                id: DeviceId("demo:maya-x-v2".into()),
                kind: DeviceKind::Mouse,
                name: model.name.into(),
                vendor: "LAMZU".into(),
                model: model.model.into(),
                vid: 0x37B0,
                pid: 0x0048,
                transport: Transport::Wireless24G,
                firmware: Some("0.0.3.0".into()),
                dongle_firmware: Some("0.0.3.0".into()),
                serial: None,
                driver: DriverKind::Lamzu,
                battery: Some(Battery {
                    percent: Some(100),
                    charging: false,
                }),
                supported: true,
                asleep: false,
                board_id: None,
            },
            state: Mutex::new(state.clone()),
            saved: Mutex::new(Default::default()),
            factory: state,
            caps,
            born: Instant::now(),
        }
    }

    fn dac() -> Self {
        let caps = dac_caps();
        Self {
            info: DeviceInfo {
                id: DeviceId("demo:tp55".into()),
                kind: DeviceKind::Other,
                name: "TP55".into(),
                vendor: "WalkPlay".into(),
                model: "tp55".into(),
                vid: 0x3302,
                pid: 0x4383,
                transport: Transport::Wired,
                firmware: Some("1.4".into()),
                dongle_firmware: None,
                serial: None,
                driver: DriverKind::Walkplay,
                battery: None,
                supported: true,
                asleep: false,
                board_id: None,
            },
            state: Mutex::new(dac_state(&caps)),
            saved: Mutex::new(Default::default()),
            factory: dac_state(&caps),
            caps,
            born: Instant::now(),
        }
    }
}

impl DeviceDriver for DemoDriver {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn caps(&self) -> Capabilities {
        self.caps.clone()
    }

    fn read(&self) -> HubResult<DeviceState> {
        std::thread::sleep(std::time::Duration::from_millis(120));
        Ok(self.state.lock().unwrap_or_else(|p| p.into_inner()).clone())
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        // 実機の HID 往復っぽい遅延。
        std::thread::sleep(std::time::Duration::from_millis(12));
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        match change.clone() {
            Change::Setting { id, value } => {
                if self.caps.setting(&id).is_none() {
                    return Err(HubError::Invalid(format!("unknown setting {id}")));
                }
                s.settings.insert(id, value);
            }
            Change::Profile(p) => {
                if p != s.profile.active {
                    // 実機と同じく構成ごとに設定の中身が入れ替わる — 現在の構成を
                    // 退避してから切替先を読み出す (未訪問の構成は工場状態)。
                    let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
                    saved.insert(s.profile.active, s.clone());
                    let mut next = saved
                        .get(&p)
                        .cloned()
                        .unwrap_or_else(|| self.factory.clone());
                    next.profile.names = s.profile.names.clone();
                    *s = next;
                    s.profile.active = p;
                }
            }
            Change::ProfileName { index, name } => {
                if let Some(n) = s.profile.names.get_mut(index as usize) {
                    *n = name;
                }
            }
            Change::DpiStages { stages, active } => {
                let m = s.mouse.as_mut().ok_or(HubError::Unsupported("mouse"))?;
                m.stages = stages;
                m.active_stage = active.min(m.stages.len().saturating_sub(1));
            }
            Change::ActiveStage(a) => {
                let m = s.mouse.as_mut().ok_or(HubError::Unsupported("mouse"))?;
                m.active_stage = a;
            }
            Change::Button { slot, action } => {
                let m = s.mouse.as_mut().ok_or(HubError::Unsupported("mouse"))?;
                if let Some(b) = m.buttons.get_mut(slot) {
                    *b = action;
                }
            }
            Change::Macro { slot, def } => {
                let list = if let Some(m) = s.mouse.as_mut() {
                    &mut m.macros
                } else if let Some(k) = s.keyboard.as_mut() {
                    &mut k.macros
                } else {
                    return Err(HubError::Unsupported("macro"));
                };
                if slot >= list.len() {
                    return Err(HubError::Invalid("macro slot".into()));
                }
                list[slot] = def;
            }
            Change::Lighting { area, state } => {
                s.lighting.insert(area, state);
            }
            Change::KeyMap { layer, key, action } => {
                let k = s
                    .keyboard
                    .as_mut()
                    .ok_or(HubError::Unsupported("keyboard"))?;
                if let Some(slot) = k
                    .keymap
                    .get_mut(layer as usize)
                    .and_then(|l| l.get_mut(key))
                {
                    *slot = action;
                }
            }
            Change::KeyMapReset { layer } => {
                let defaults = keyboard_state(&self.caps);
                let k = s
                    .keyboard
                    .as_mut()
                    .ok_or(HubError::Unsupported("keyboard"))?;
                if let (Some(dst), Some(src)) = (
                    k.keymap.get_mut(layer as usize),
                    defaults
                        .keyboard
                        .as_ref()
                        .and_then(|d| d.keymap.get(layer as usize)),
                ) {
                    *dst = src.clone();
                }
            }
            Change::KeyPerf { keys, perf } => {
                let k = s
                    .keyboard
                    .as_mut()
                    .ok_or(HubError::Unsupported("keyboard"))?;
                for i in keys {
                    if let Some(p) = k.perf.get_mut(i) {
                        *p = perf;
                    }
                }
            }
            Change::Advanced {
                key,
                layer,
                binding,
            } => {
                let k = s
                    .keyboard
                    .as_mut()
                    .ok_or(HubError::Unsupported("keyboard"))?;
                k.advanced.retain(|a| !(a.key == key && a.layer == layer));
                if let Some(b) = binding {
                    k.advanced.push(AdvancedKey {
                        key,
                        layer,
                        binding: b,
                    });
                }
            }
            Change::Peq { band, value } => {
                let n = self.caps.peq.as_ref().map(|p| p.bands).unwrap_or(0) as usize;
                if let Some(peq) = s.peq.as_mut().filter(|p| band < p.len() && band < n) {
                    peq[band] = value;
                }
            }
            Change::PeqAll { bands } => {
                if s.peq.is_some() {
                    s.peq = Some(bands);
                }
            }
            Change::Adaptive(curve) => {
                if let Some(m) = s.mouse.as_mut().filter(|m| m.adaptive.is_some()) {
                    m.adaptive = Some(curve);
                }
            }
        }
        Ok(())
    }

    fn run_action(&self, id: &str, file: Option<&Path>) -> HubResult<ActionOutcome> {
        std::thread::sleep(std::time::Duration::from_millis(600));
        let msg = match id {
            "reset_profile" | "factory_reset" | "reset_keymap" | "reset_perf"
            | "reset_lighting" => {
                let fresh = if self.caps.keyboard.is_some() {
                    keyboard_state(&self.caps)
                } else {
                    mouse_state(&self.caps)
                };
                *self.state.lock().unwrap_or_else(|p| p.into_inner()) = fresh;
                return Ok(ActionOutcome {
                    message: Some("初期設定に戻しました (デモ)".into()),
                    reload: true,
                });
            }
            "pair" => "ペアリングモードに入りました (デモ)".to_string(),
            "calibrate" => {
                "キャリブレーションを開始しました。全キーを底まで押してください (デモ)".to_string()
            }
            "calibrate_finish" => "キャリブレーションを保存しました (デモ)".to_string(),
            "eq_def" => {
                // walkplay と同じ DEF プリセット 8 バンド分。
                const DEF_FREQS: [u16; 8] = [50, 200, 500, 1000, 2500, 5000, 10000, 15000];
                let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(peq) = st.peq.as_mut() {
                    for (i, f) in DEF_FREQS.iter().enumerate() {
                        if let Some(b) = peq.get_mut(i) {
                            *b = PeqBand {
                                freq: *f,
                                q: 0.75,
                                gain: 0.,
                                filter: PeqFilter::Peak,
                            };
                        }
                    }
                }
                return Ok(ActionOutcome {
                    message: Some("DEF プリセットを適用しました (デモ)".into()),
                    reload: true,
                });
            }
            "firmware" => {
                for i in 0..=20 {
                    emit(HubEvent::Progress {
                        id: self.info.id.clone(),
                        label: "ファームウェア書き込み (デモ)".into(),
                        fraction: Some(i as f32 / 20.),
                        done: false,
                    });
                    std::thread::sleep(std::time::Duration::from_millis(120));
                }
                emit(HubEvent::Progress {
                    id: self.info.id.clone(),
                    label: String::new(),
                    fraction: None,
                    done: true,
                });
                format!(
                    "ファームウェア更新はデモでは行いません: {}",
                    file.map(|p| p.display().to_string()).unwrap_or_default()
                )
            }
            _ => return Err(HubError::Unsupported("action")),
        };
        Ok(ActionOutcome {
            message: Some(msg),
            reload: false,
        })
    }

    fn battery(&self) -> HubResult<Battery> {
        self.info.battery.ok_or(HubError::Unsupported("battery"))
    }

    /// DirectDrive 対応アリの caps を出すので no-op で成功を返す (送信先が無いだけ)。
    fn direct_drive(&self, _area: u8, _cells: &BTreeMap<usize, Rgb>) -> HubResult<()> {
        Ok(())
    }

    fn direct_drive_stop(&self, _area: u8) -> HubResult<()> {
        Ok(())
    }

    fn live_travel(&self) -> HubResult<Vec<f32>> {
        let n = self
            .caps
            .keyboard
            .as_ref()
            .map(|k| k.layout.len())
            .unwrap_or(0);
        let t = self.born.elapsed().as_secs_f32();
        // WASD がリズムよく沈む擬似データ。
        Ok((0..n)
            .map(|i| {
                let phase = match i {
                    16 => 0.0, // W
                    29 => 1.3, // A
                    30 => 2.1, // S
                    31 => 2.9, // D
                    _ => return 0.0,
                };
                (((t * 2.2 + phase).sin() * 0.5 + 0.5).powf(1.6) * 4.0).min(4.0)
            })
            .collect())
    }

    fn live_buttons(&self) -> HubResult<Vec<ButtonDepth>> {
        let n = self
            .caps
            .mouse
            .as_ref()
            .map(|m| m.live_buttons.len())
            .unwrap_or(0);
        if n == 0 {
            return Err(HubError::Unsupported("live_buttons"));
        }
        let t = self.born.elapsed().as_secs_f32();
        Ok((0..n)
            .map(|i| {
                let depth = ((t * 1.4 + i as f32 * 1.1).sin() * 0.5 + 0.5).powf(1.3);
                ButtonDepth {
                    depth,
                    trigger: Some(0.5),
                    pressed: Some(depth >= 0.5),
                }
            })
            .collect())
    }

    fn live_actuation(&self) -> HubResult<Vec<f32>> {
        let st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        Ok(st
            .keyboard
            .as_ref()
            .map(|k| {
                k.perf
                    .iter()
                    .map(|p| match p.mode {
                        TriggerMode::Rapid => p.rt_first,
                        TriggerMode::Normal => p.actuation,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn live_axis(&self) -> HubResult<Vec<AxisSample>> {
        Ok(self
            .live_travel()?
            .into_iter()
            .map(|r| AxisSample {
                route: r,
                adc: (2400.0 - r * 300.0) as u16,
                status: u8::from(r > 0.2),
                calibrated: 2,
            })
            .collect())
    }
}

// ====================================================================== マウス (mouse.fit 相当)

fn opts(v: &[(i64, &str)]) -> Vec<ChoiceOpt> {
    v.iter().map(|(a, b)| ChoiceOpt::new(*a, *b)).collect()
}

fn macro_caps_mouse() -> MacroCaps {
    MacroCaps {
        slots: 3,
        max_events: 70,
        max_delay_ms: 65_535,
        modes: opts(&[
            (0, "指定回数実行"),
            (1, "離すまで繰り返す"),
            (2, "もう一度押すまで繰り返す"),
        ]),
        repeat_modes: vec![0],
        names: true,
        mouse_events: true,
    }
}

pub fn mouse_caps(wireless: bool, magnetic: bool) -> Capabilities {
    use section::*;
    use SettingValue::*;
    let mut settings = vec![SettingSpec::choice(
        "polling_wired",
        PERFORMANCE,
        "ポーリングレート (有線)",
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
        SettingSpec::toggle("hyper_mode", PERFORMANCE, "高速モード")
            .help("性能を優先します。消費電力が増えます (2K/4K/8K と併用可)"),
        SettingSpec::toggle("competitive", PERFORMANCE, "競技モード")
            .help("センサーを常時最高性能で動かします"),
        SettingSpec::toggle("long_distance", PERFORMANCE, "長距離モード")
            .help("レシーバーとの距離が離れていても安定させます"),
        SettingSpec::choice(
            "lod",
            SENSOR,
            "リフトオフ距離",
            opts(&[
                (7, "0.7mm"),
                (9, "0.9mm"),
                (12, "1.2mm"),
                (14, "1.4mm"),
                (16, "1.6mm"),
            ]),
        )
        .help("マウスを持ち上げたとき反応しなくなる高さ"),
        SettingSpec::toggle("motion_sync", SENSOR, "モーション同期").help(
            "ポインタとマウスの動きを同期させ、軌跡を滑らかにします (わずかに遅延が増えます)",
        ),
        SettingSpec::toggle("angle_snap", SENSOR, "直線補正").help("移動のブレを直線に補正します"),
        SettingSpec::toggle("ripple", SENSOR, "リップル制御").help("高DPI時の細かい揺れを抑えます"),
        SettingSpec::slider("sensor_angle", SENSOR, "センサー角度", -30., 30., 1., "°")
            .help("持ち方に合わせて座標軸を回転させます"),
        SettingSpec::toggle("sensor_fps_20k", SENSOR, "20K FPS")
            .help("センサーのフレームレートを 20000 に固定します"),
        SettingSpec::toggle("sensor_center", SENSOR, "センサー仮想中心")
            .help("センサー位置を本体中央として扱います"),
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
        SettingSpec::toggle("spdt_left", CLICK, "Left SPDT")
            .help("光学/機械スイッチの SPDT 方式でチャタリングを防ぎます"),
        SettingSpec::toggle("spdt_right", CLICK, "Right SPDT"),
        SettingSpec::toggle("liftoff_debounce", CLICK, "ボタン誤操作防止")
            .help("持ち上げた瞬間のクリックを無視します"),
        SettingSpec::toggle("combine_key", CLICK, "組み合わせキー機能")
            .help("本体のボタン同時押しで設定を変更します。オフにするとペアリング切替も無効"),
    ]);
    if magnetic {
        settings.extend([
            SettingSpec::slider(
                "mag_left_trigger",
                MAGNETIC,
                "Left トリガー位置",
                0.1,
                1.0,
                0.05,
                "mm",
            ),
            SettingSpec::slider(
                "mag_right_trigger",
                MAGNETIC,
                "Right トリガー位置",
                0.1,
                1.0,
                0.05,
                "mm",
            ),
            SettingSpec::toggle("mag_quick_trigger", MAGNETIC, "クイックトリガー")
                .help("出荷時の校正値です。変更すると連打/取りこぼしの原因になることがあります"),
        ]);
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
    settings.push(
        SettingSpec::choice(
            "dpi_led_effect",
            POWER,
            "インジケーター表示",
            opts(&[(0, "常時点灯"), (1, "呼吸"), (2, "切替時のみ")]),
        )
        .when("dpi_indicator", Bool(true)),
    );

    let mut actions = vec![
        ActionSpec::new("reset_profile", section::SYSTEM, "設定をリセット")
            .help("このプロファイルの設定を初期値に戻します")
            .danger("現在の設定を初期値に戻します。よろしいですか?"),
    ];
    if wireless {
        actions.push(
            ActionSpec::new("pair", section::CONNECTION, "レシーバーとペアリング")
                .help("レシーバーを挿し、マウス底面のスイッチを 2.4G にしてから実行")
                .confirm("ペアリングモードに入ります。"),
        );
    }
    if magnetic {
        actions.push(
            ActionSpec::new("calibrate", section::MAGNETIC, "L/R ボタンの校正")
                .help("有線で接続し、L/R ボタンを底まで押して 2 秒保持 → 離す")
                .confirm("校正を開始します。L/R ボタンを触らずに待ってください。"),
        );
    }
    actions.push(
        ActionSpec::new("firmware", section::SYSTEM, "ファームウェア更新")
            .help(".bin を選んで書き込みます")
            .file(&["bin"])
            .danger("書き込み中はケーブルを抜かないでください。続けますか?"),
    );

    Capabilities {
        settings,
        actions,
        profiles: None,
        mouse: Some(MouseCaps {
            dpi_min: 50,
            dpi_max: 40_000,
            dpi_step: 10,
            min_stages: 1,
            max_stages: 7,
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
                ButtonActionKind::ScrollModeToggle,
                ButtonActionKind::Disabled,
            ],
            button_options: None,
            require_left_click: true,
            macros: Some(macro_caps_mouse()),
            live_buttons: if magnetic { vec![0, 1] } else { vec![] },
            adaptive_dpi: None,
        }),
        keyboard: None,
        lighting: vec![],
        battery: wireless,
        peq: None,
        firmware_feed: None,
    }
}

fn mouse_state(caps: &Capabilities) -> DeviceState {
    use SettingValue::*;
    let colors = [
        0xff3040, 0x2f6bff, 0x22d36b, 0xff3df5, 0xffd21a, 0xffffff, 0x2ee6ff,
    ];
    let dpis = [400, 800, 1600, 3200];
    let mut settings = BTreeMap::new();
    for s in &caps.settings {
        let v = match s.id.as_str() {
            "polling_wired" | "polling_wireless" => Choice(1000),
            "lod" => Choice(7),
            "debounce_press" | "debounce_release" => Num(2.),
            "wheel_debounce" => Num(10.),
            "sensor_angle" => Num(0.),
            "sleep_min" => Choice(1),
            "mag_left_trigger" | "mag_right_trigger" => Num(0.5),
            "dpi_led_effect" => Choice(0),
            "motion_sync" | "dpi_indicator" | "combine_key" => Bool(true),
            _ => match s.kind {
                SettingKind::Toggle => Bool(false),
                SettingKind::Slider { min, .. } => Num(min),
                SettingKind::Choice { ref options } => {
                    Choice(options.first().map(|o| o.value).unwrap_or(0))
                }
            },
        };
        settings.insert(s.id.clone(), v);
    }
    let m = caps.mouse.as_ref().expect("mouse caps");
    DeviceState {
        profile: ProfileState::default(),
        settings,
        mouse: Some(MouseState {
            stages: dpis
                .iter()
                .enumerate()
                .map(|(i, d)| DpiStage {
                    x: *d,
                    y: None,
                    color: Rgb::hex(colors[i]),
                    enabled: true,
                })
                .collect(),
            active_stage: 1,
            buttons: vec![
                ButtonAction::Mouse(MouseFn::Left),
                ButtonAction::Mouse(MouseFn::Right),
                ButtonAction::Mouse(MouseFn::Middle),
                ButtonAction::Mouse(MouseFn::Back),
                ButtonAction::Mouse(MouseFn::Forward),
            ],
            macros: {
                let mut v: Vec<Option<MacroDef>> =
                    vec![None; m.macros.as_ref().map(|c| c.slots as usize).unwrap_or(0)];
                if let Some(first) = v.first_mut() {
                    *first = Some(MacroDef {
                        name: "ジャンプ投げ".into(),
                        events: vec![
                            MacroEvent {
                                input: MacroInput::Key(0x2C),
                                edge: MacroEdge::Down,
                                delay_ms: 8,
                            },
                            MacroEvent {
                                input: MacroInput::Mouse(MouseFn::Left),
                                edge: MacroEdge::Up,
                                delay_ms: 15,
                            },
                            MacroEvent {
                                input: MacroInput::Key(0x2C),
                                edge: MacroEdge::Up,
                                delay_ms: 0,
                            },
                        ],
                        mode: 0,
                        repeat: 1,
                    });
                }
                v
            },
            adaptive: None,
        }),
        keyboard: None,
        lighting: BTreeMap::new(),
        peq: None,
    }
}

// ====================================================================== キーボード (hub.xsyd.top 相当)

/// xsyd のキーコード空間 (keyTip_* の番号) に合わせた候補表。
/// 基本キーは HID usage をそのまま使う (xsyd も下位 16bit が usage)。
pub fn xsyd_catalog() -> Vec<KeyGroup> {
    let g = |name: &str, v: &[(u32, &str, &str)]| KeyGroup {
        name: name.into(),
        keys: v
            .iter()
            .map(|(c, l, s)| KeyChoice {
                code: *c,
                label: (*l).into(),
                short: Some((*s).into()),
            })
            .collect(),
    };
    vec![
        keys::basic_group(),
        g(
            "マウス",
            &[
                (16640, "マウス左", "M左"),
                (16896, "マウス右", "M右"),
                (17152, "マウス中", "M中"),
                (17408, "マウス進む", "M進"),
                (17664, "マウス戻る", "M戻"),
                (18945, "ホイール上", "W↑"),
                (19201, "ホイール下", "W↓"),
                (17920, "カーソル左", "M←"),
                (18176, "カーソル右", "M→"),
                (18432, "カーソル上", "M↑"),
                (18688, "カーソル下", "M↓"),
            ],
        ),
        g(
            "メディア",
            &[
                (0x20CD, "再生/一時停止", "⏯"),
                (8373, "次の曲", "⏭"),
                (8374, "前の曲", "⏮"),
                (0x20E9, "音量 +", "🔊"),
                (0x20EA, "音量 −", "🔉"),
                (8418, "ミュート", "🔇"),
                (8303, "画面 明るさ +", "☀+"),
                (8304, "画面 明るさ −", "☀−"),
                (8586, "メール", "Mail"),
                (8596, "マイコンピューター", "PC"),
                (8739, "ブラウザ", "Web"),
            ],
        ),
        g(
            "ライティング",
            &[
                (62208, "ライティングモード切替", "L模"),
                (62214, "前のライティングモード", "L前"),
                (62209, "ライティング色切替", "L色"),
                (62210, "明るさ +", "L+"),
                (62211, "明るさ −", "L−"),
                (62212, "速度 +", "S+"),
                (62213, "速度 −", "S−"),
                (62215, "ライト ON/OFF", "L⏻"),
                (62216, "ライティング方向", "L⇄"),
                (62224, "デコライト モード切替", "D模"),
                (62231, "デコライト ON/OFF", "D⏻"),
            ],
        ),
        g(
            "レイヤー",
            &[
                (61696, "メインレイヤーへ", "L0"),
                (61697, "Fn1 レイヤーへ", "Fn1"),
                (61698, "Fn2 レイヤーへ", "Fn2"),
                (61699, "Fn3 レイヤーへ", "Fn3"),
            ],
        ),
        g(
            "システム",
            &[
                (61957, "構成 1 に切替", "P1"),
                (61958, "構成 2 に切替", "P2"),
                (61959, "構成 3 に切替", "P3"),
                (61960, "構成 4 に切替", "P4"),
                (61961, "Win キーロック", "WinL"),
                (61953, "Windows 配列 (長押し)", "Win"),
                (61954, "Mac 配列 (長押し)", "Mac"),
                (61955, "磁軸キャリブレーション (長押し)", "Cal"),
                (61952, "初期化 (長押し)", "Rst"),
            ],
        ),
    ]
}

pub fn keyboard_caps() -> Capabilities {
    use section::*;
    let layout = registry::su66max_layout();
    // xsyd.rs と同一 (実機カメラ検証済みの対応 — docs/PROTOCOL_NOTES.md)。
    let effects = vec![
        LightEffect::new(0, "固定色").color().still(),
        LightEffect::new(1, "虹の横ながれ").color().dir(),
        LightEffect::new(2, "ゆっくり変色").color(),
        LightEffect::new(3, "すばやく変色").color(),
        LightEffect::new(4, "虹の市松").color(),
        LightEffect::new(5, "虹のめぐり").color(),
        LightEffect::new(6, "虹の明滅").color(),
        LightEffect::new(7, "ほのかな窓").color().dir(),
        LightEffect::new(8, "ゆるやか変形").color(),
        LightEffect::new(9, "色帯シフト").color().dir(),
        LightEffect::new(10, "虹のうねり").color().dir(),
        LightEffect::new(11, "斜めの光帯").color().dir(),
        LightEffect::new(12, "コンフェッティ").color(),
        LightEffect::new(13, "虹のモザイク").color(),
        LightEffect::new(14, "光の雨").color().dir(),
        LightEffect::new(15, "跳ねる光").color(),
        LightEffect::new(16, "カスタム").per_key(),
        LightEffect::new(17, "押下・まわり").color(),
        LightEffect::new(18, "押下・行").color(),
        LightEffect::new(19, "押下・広域").color(),
    ];
    let decor = vec![
        LightEffect::new(0, "スタティック").color().still(),
        LightEffect::new(1, "呼吸").color(),
        LightEffect::new(2, "スペクトラム"),
        LightEffect::new(3, "流れる").dir(),
    ];
    let polling = opts(&[
        (8000, "8K"),
        (4000, "4K"),
        (2000, "2K"),
        (1000, "1K"),
        (500, "500"),
        (250, "250"),
        (125, "125"),
    ]);
    Capabilities {
        settings: vec![
            SettingSpec::choice("polling", PERFORMANCE, "ポーリングレート", polling)
                .help("キー入力をPCへ報告する頻度 (Hz)"),
            SettingSpec::choice(
                "usb_mode",
                PERFORMANCE,
                "USB モード",
                opts(&[
                    (0, "USB 2.0"),
                    (1, "USB 3.0 · 8K"),
                    (2, "USB 3.0 · 16K"),
                    (3, "USB 3.0 · 32K"),
                ]),
            )
            .help("3.0 モードは専用ドライバが必要です")
            .confirm("USB モードを変更するとキーボードが再接続されます。続けますか?"),
            SettingSpec::choice(
                "system_type",
                KEYBOARD,
                "OS 配列",
                opts(&[(0, "Windows"), (1, "macOS")]),
            ),
            SettingSpec::toggle("win_lock", KEYBOARD, "Win キーロック")
                .help("ゲーム中の Win キー誤爆を防ぎます"),
            SettingSpec::toggle("shake_opt", KEYBOARD, "チャタリング防止最適化")
                .help("無効にすると極端な設定でキーが意図せず反応する場合があります")
                .confirm("無効にすると誤入力が起きる場合があります。続けますか?"),
            SettingSpec::toggle("compat_mode", KEYBOARD, "互換モード")
                .help("BIOS や一部の環境で認識されないときに使います"),
            SettingSpec::choice(
                "light_sleep",
                LIGHTING,
                "ライティング自動消灯",
                opts(&[
                    (0, "しない"),
                    (1, "1分"),
                    (3, "3分"),
                    (5, "5分"),
                    (10, "10分"),
                    (30, "30分"),
                ]),
            ),
            SettingSpec::toggle("light_link", LIGHTING, "ライティング連動")
                .help("本体とデコレーションライトを同じエフェクトにします"),
        ],
        actions: vec![
            ActionSpec::new("calibrate", section::KEYBOARD, "磁軸キャリブレーション")
                .help("開始後、すべてのキーを 1 回ずつ底まで押してください"),
            ActionSpec::new(
                "calibrate_finish",
                section::KEYBOARD,
                "キャリブレーションを保存",
            ),
            ActionSpec::new("reset_perf", section::SYSTEM, "トリガー設定を初期化")
                .danger("全キーのトリガー設定を初期値に戻します。"),
            ActionSpec::new("reset_keymap", section::SYSTEM, "キー割り当てを初期化")
                .danger("全レイヤーのキー割り当てを初期値に戻します。"),
            ActionSpec::new("reset_lighting", section::SYSTEM, "ライティングを初期化")
                .danger("ライティングを初期値に戻します。"),
            ActionSpec::new("factory_reset", section::SYSTEM, "工場出荷状態に戻す")
                .danger("すべての構成・マクロ・キャリブレーションが消去されます。よろしいですか?"),
            ActionSpec::new("firmware", section::SYSTEM, "ファームウェア更新")
                .file(&["bin"])
                .danger("更新中は接続を切らないでください。続けますか?"),
        ],
        profiles: Some(ProfileCaps {
            count: 4,
            renamable: true,
        }),
        mouse: None,
        keyboard: Some(KeyboardCaps {
            layout,
            layers: vec!["メイン".into(), "Fn".into()],
            catalog: xsyd_catalog(),
            combos: true,
            perf: Some(PerfCaps {
                travel: 4.0,
                min: 0.01,
                step: 0.01,
                rapid_trigger: true,
                separate_release: true,
                rt_first: true,
                deadzones: true,
                continuous_rt: true,
                switch_types: vec![
                    "既定 (磁軸 A)".into(),
                    "磁軸 B (軽)".into(),
                    "磁軸 C (リニア長)".into(),
                ],
                switch_ids: vec![],
                per_key: true,
                live_travel: true,
            }),
            advanced: vec![
                AdvancedKind::Dks,
                AdvancedKind::Mpt,
                AdvancedKind::Mt,
                AdvancedKind::Tgl,
                AdvancedKind::End,
                AdvancedKind::Socd,
                AdvancedKind::Rs,
            ],
            macros: Some(MacroCaps {
                slots: 16,
                max_events: 128,
                max_delay_ms: 32_768,
                modes: opts(&[
                    (0, "クリックで指定回数実行"),
                    (1, "押している間リピート (離すと即停止)"),
                    (2, "押している間リピート (完了後に停止)"),
                    (3, "クリックで開始 / もう一度で停止"),
                ]),
                repeat_modes: vec![0],
                names: false,
                mouse_events: false,
            }),
        }),
        lighting: vec![
            LightArea {
                id: 0,
                name: "メインキーボード".into(),
                effects,
                brightness_max: 4,
                speed_max: 4,
                directions: vec!["正方向".into(), "逆方向".into()],
                switchable: true,
                zones: vec!["上部ライト".into(), "下部ライト".into()],
                per_key: true,
                direct_drive: true,
                live_readback: false,
            },
            LightArea {
                id: 1,
                name: "デコレーションライト".into(),
                effects: decor,
                brightness_max: 4,
                speed_max: 4,
                directions: vec!["正方向".into(), "逆方向".into()],
                switchable: true,
                zones: vec![],
                per_key: false,
                direct_drive: false,
                live_readback: false,
            },
        ],
        battery: false,
        peq: None,
        firmware_feed: None,
    }
}

fn keyboard_state(caps: &Capabilities) -> DeviceState {
    let k = caps.keyboard.as_ref().expect("keyboard caps");
    let n = k.layout.len();
    // 刻印 → HID usage (既定キーマップ)。
    let label_usage = |label: &str| -> KeyAction {
        let special = match label {
            "Esc" => Some(0x29),
            "Backspace" => Some(0x2A),
            "Tab" => Some(0x2B),
            "Caps" => Some(0x39),
            "Enter" => Some(0x28),
            "LShift" => Some(0xE1),
            "RShift" => Some(0xE5),
            "LCtrl" => Some(0xE0),
            "LWin" => Some(0xE3),
            "LAlt" => Some(0xE2),
            "RAlt" => Some(0xE6),
            "Space" => Some(0x2C),
            "Del" => Some(0x4C),
            "↑" => Some(0x52),
            "↓" => Some(0x51),
            "←" => Some(0x50),
            "→" => Some(0x4F),
            "Fn" => return KeyAction::Code(61697),
            _ => None,
        };
        let usage = special.or_else(|| keys::HID_KEYS.iter().find(|h| h.2 == label).map(|h| h.0));
        usage
            .map(|u| KeyAction::Code(u as u32))
            .unwrap_or(KeyAction::None)
    };
    let main: Vec<KeyAction> = k.layout.iter().map(|l| label_usage(&l.label)).collect();
    // Fn レイヤー: 数字段 → F1..F12、矢印 → Home/End/PgUp/PgDn、他は透過。
    let fnl: Vec<KeyAction> = k
        .layout
        .iter()
        .map(|l| match l.label.as_str() {
            "1" => KeyAction::Code(0x3A),
            "2" => KeyAction::Code(0x3B),
            "3" => KeyAction::Code(0x3C),
            "4" => KeyAction::Code(0x3D),
            "5" => KeyAction::Code(0x3E),
            "6" => KeyAction::Code(0x3F),
            "7" => KeyAction::Code(0x40),
            "8" => KeyAction::Code(0x41),
            "9" => KeyAction::Code(0x42),
            "0" => KeyAction::Code(0x43),
            "-" => KeyAction::Code(0x44),
            "=" => KeyAction::Code(0x45),
            "↑" => KeyAction::Code(0x4B),
            "↓" => KeyAction::Code(0x4E),
            "←" => KeyAction::Code(0x4A),
            "→" => KeyAction::Code(0x4D),
            "Esc" => KeyAction::Code(0x35),
            "Fn" => KeyAction::Code(61697),
            "LWin" => KeyAction::Code(61961),
            "Q" => KeyAction::Code(61957),
            "W" => KeyAction::Code(61958),
            "E" => KeyAction::Code(61959),
            "R" => KeyAction::Code(61960),
            _ => KeyAction::Transparent,
        })
        .collect();
    let mut perf = vec![KeyPerf::default(); n];
    // WASD は RT の見本。
    for (i, l) in k.layout.iter().enumerate() {
        if matches!(l.label.as_str(), "W" | "A" | "S" | "D") {
            perf[i] = KeyPerf {
                mode: TriggerMode::Rapid,
                actuation: 0.4,
                rt_first: 0.2,
                rt_press: 0.05,
                rt_release: 0.05,
                ..KeyPerf::default()
            };
        }
    }
    let idx = |label: &str| k.layout.iter().position(|l| l.label == label).unwrap_or(0);
    let mut lighting = BTreeMap::new();
    lighting.insert(
        0,
        LightingState {
            on: true,
            effect: 14,
            color: Rgb::hex(0x7ad7ff),
            brightness: 75,
            speed: 50,
            direction: 0,
            per_key: BTreeMap::new(),
            zone_on: vec![true, true],
            palette: vec![
                Rgb::hex(0x7ad7ff),
                Rgb::hex(0xff5c7a),
                Rgb::hex(0x5cff8a),
                Rgb::hex(0xffd25c),
                Rgb::hex(0x8a7aff),
                Rgb::hex(0xff8a3c),
                Rgb::hex(0x3cffe0),
                Rgb::hex(0xffffff),
            ],
            palette_sel: 0,
            color_correction: Some(Rgb::new(100, 100, 100)),
        },
    );
    lighting.insert(
        1,
        LightingState {
            on: true,
            effect: 1,
            color: Rgb::hex(0xa98bff),
            brightness: 50,
            speed: 50,
            direction: 0,
            per_key: BTreeMap::new(),
            zone_on: vec![],
            palette: vec![Rgb::hex(0xa98bff)],
            palette_sel: 0,
            color_correction: Some(Rgb::new(100, 100, 100)),
        },
    );
    let mut settings = BTreeMap::new();
    for s in &caps.settings {
        settings.insert(
            s.id.clone(),
            match s.id.as_str() {
                "polling" => SettingValue::Choice(8000),
                "shake_opt" => SettingValue::Bool(true),
                _ => match &s.kind {
                    SettingKind::Toggle => SettingValue::Bool(false),
                    SettingKind::Slider { min, .. } => SettingValue::Num(*min),
                    SettingKind::Choice { options } => {
                        SettingValue::Choice(options.first().map(|o| o.value).unwrap_or(0))
                    }
                },
            },
        );
    }
    let slots = k.macros.as_ref().map(|m| m.slots as usize).unwrap_or(0);
    DeviceState {
        profile: ProfileState {
            active: 0,
            names: vec![
                "VALORANT".into(),
                "FPS 汎用".into(),
                "タイピング".into(),
                String::new(),
            ],
        },
        settings,
        mouse: None,
        keyboard: Some(KeyboardState {
            keymap: vec![main, fnl],
            perf,
            advanced: vec![
                AdvancedKey {
                    key: idx("A"),
                    layer: 0,
                    binding: Advanced::Socd {
                        other: idx("D"),
                        code_a: 0x04,
                        code_b: 0x07,
                        mode: SocdMode::Last,
                        delay_ms: 0,
                    },
                },
                AdvancedKey {
                    key: idx("LShift"),
                    layer: 0,
                    binding: Advanced::Mt {
                        tap: 0x29,
                        hold: 0xE1,
                        hold_ms: 200,
                    },
                },
            ],
            macros: vec![None; slots],
        }),
        lighting,
        peq: None,
    }
}

// ====================================================================== USB DAC (peq.szwalkplay.com 相当)

fn dac_caps() -> Capabilities {
    Capabilities {
        settings: vec![
            SettingSpec::choice(
                "dac_filter",
                section::AUDIO,
                "DAC デジタルフィルター",
                opts(&[
                    (0, "FAST-LL"),
                    (1, "FAST-PC"),
                    (2, "SLOW-LL"),
                    (3, "SLOW-PC"),
                    (4, "NON-OS"),
                ]),
            ),
            SettingSpec::choice(
                "dac_work_mode",
                section::AUDIO,
                "DAC ワークモード",
                opts(&[(0, "Class-H"), (1, "Class-AB")]),
            ),
            SettingSpec::choice(
                "output_gain",
                section::AUDIO,
                "出力ゲインモード",
                opts(&[(0, "Low"), (1, "High")]),
            ),
            SettingSpec::slider(
                "channel_balance",
                section::AUDIO,
                "チャンネルバランス",
                -127.,
                127.,
                1.,
                "",
            )
            .help("正 = 右に寄せる (左を減衰)。0 で中央。"),
            SettingSpec::slider(
                "eq_volume",
                section::AUDIO,
                "EQ ボリューム",
                -16.,
                6.,
                1.,
                "dB",
            ),
            SettingSpec::slider(
                "mic_offset",
                section::AUDIO,
                "マイクオフセット",
                -15.,
                15.,
                1.,
                "",
            ),
        ],
        actions: vec![
            ActionSpec::new("eq_def", section::AUDIO, "EQ を工場出荷値 (DEF) に戻す")
                .help("8 バンドの公式 DEF プリセットを書き込みます")
                .confirm("EQ を DEF プリセットに戻します。よろしいですか?"),
        ],
        profiles: None,
        mouse: None,
        keyboard: None,
        lighting: vec![],
        battery: false,
        peq: Some(PeqCaps {
            bands: 10,
            freq_min: 20,
            freq_max: 20_000,
            q_min: 0.5,
            q_max: 10.,
            gain_min: -12.,
            gain_max: 12.,
            filters: vec![
                PeqFilter::LowShelf,
                PeqFilter::Peak,
                PeqFilter::HighShelf,
                PeqFilter::LowPass,
                PeqFilter::HighPass,
            ],
        }),
        firmware_feed: None,
    }
}

fn dac_state(caps: &Capabilities) -> DeviceState {
    let mut settings = BTreeMap::new();
    for s in &caps.settings {
        let v = match s.id.as_str() {
            "dac_filter" => SettingValue::Choice(2),
            "output_gain" => SettingValue::Choice(1),
            "eq_volume" => SettingValue::Num(-1.),
            _ => match s.kind {
                SettingKind::Toggle => SettingValue::Bool(false),
                SettingKind::Slider { .. } => SettingValue::Num(0.),
                SettingKind::Choice { ref options } => {
                    SettingValue::Choice(options.first().map(|o| o.value).unwrap_or(0))
                }
            },
        };
        settings.insert(s.id.clone(), v);
    }
    let freqs = [31u16, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
    DeviceState {
        profile: ProfileState::default(),
        settings,
        mouse: None,
        keyboard: None,
        lighting: BTreeMap::new(),
        peq: Some(
            freqs
                .iter()
                .map(|&f| PeqBand {
                    freq: f,
                    q: 0.75,
                    gain: 0.,
                    filter: PeqFilter::Peak,
                })
                .collect(),
        ),
    }
}

// ====================================================================== LAMZU (公式 Aurora 相当)

fn lamzu_state(caps: &Capabilities) -> DeviceState {
    let mut settings = BTreeMap::new();
    for (id, v) in [
        ("polling_wireless", SettingValue::Choice(8000)),
        ("hyper_mode", SettingValue::Bool(true)),
        ("tracking_mode", SettingValue::Bool(true)),
        ("lod", SettingValue::Choice(7)),
        ("glass_mode", SettingValue::Bool(false)),
        ("motion_sync", SettingValue::Bool(false)),
        ("angle_snap", SettingValue::Bool(false)),
        ("ripple", SettingValue::Bool(false)),
        ("dpi_xy", SettingValue::Bool(false)),
        ("angle_tune", SettingValue::Num(1.)),
        ("sensor_offset", SettingValue::Num(0.)),
        ("debounce", SettingValue::Num(0.)),
        ("rapid_left", SettingValue::Bool(true)),
        ("rapid_right", SettingValue::Bool(false)),
        ("bhop_dir", SettingValue::Choice(0)),
        ("bhop_ms", SettingValue::Choice(200)),
        ("sleep_min", SettingValue::Choice(1)),
        ("dongle_led", SettingValue::Bool(true)),
        ("dongle_led1", SettingValue::Choice(10)),
        ("dongle_led2", SettingValue::Choice(11)),
        ("dongle_led3", SettingValue::Choice(6)),
    ] {
        if caps.setting(id).is_some() {
            settings.insert(id.to_string(), v);
        }
    }
    let colors = [0xD00067u32, 0x00A5D1, 0x0F9B00, 0xFFFFFF, 0xCAC700];
    let stages = [400u32, 800, 1600, 3200, 6400]
        .iter()
        .zip(colors)
        .map(|(&x, c)| DpiStage {
            x,
            y: None,
            color: Rgb::hex(c),
            enabled: true,
        })
        .collect();
    DeviceState {
        profile: ProfileState::default(),
        settings,
        mouse: Some(MouseState {
            stages,
            active_stage: 2,
            buttons: vec![
                ButtonAction::Mouse(MouseFn::Left),
                ButtonAction::Mouse(MouseFn::Right),
                ButtonAction::Mouse(MouseFn::Middle),
                ButtonAction::Mouse(MouseFn::Back),
                ButtonAction::Mouse(MouseFn::Forward),
            ],
            macros: vec![],
            adaptive: Some(AdaptiveCurve {
                enabled: false,
                points: crate::lamzu::adaptive_presets()[0].1.clone(),
            }),
        }),
        keyboard: None,
        lighting: BTreeMap::new(),
        peq: None,
    }
}

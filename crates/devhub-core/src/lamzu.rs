//! LAMZU Aurora (www.lamzu.net) LM20 世代の機種表と Capabilities。
//!
//! ドライバ (`devhub-hid::lamzu`) とデモ (`demo.rs`) の両方が使う。
//! 値はすべて公式 `Config/env-models.json` (v1.0.38) と設定画面
//! `index-Bn9q780G.js` の LM20 経路 (`T2()`) から取っている
//! (docs/LAMZU_MAYA_X_V2_AUDIT2.md)。表示名は公式 Aurora の日本語表示に合わせる。

use crate::model::*;

/// LM20 世代の 1 機種。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LamzuModel {
    /// 有線 PID (ドングル経由ではペア先 PID として返る)。
    pub pid: u16,
    /// 製品名 (公式製品ページの表記)。
    pub name: &'static str,
    /// 内部モデル id。
    pub model: &'static str,
    /// 公式機種定義の行名。
    pub row: &'static str,
    pub sensor_offset: bool,
    pub adaptive_dpi: bool,
    pub glass_mode: bool,
}

/// 公式機種定義の LM20 行 (Mini は受信機が 0054)。
pub const MODELS: &[LamzuModel] = &[
    LamzuModel {
        pid: 0x0044,
        name: "LAMZU MAYA X V2",
        model: "maya-x-v2",
        row: "Maya X(LM20)",
        sensor_offset: true,
        adaptive_dpi: true,
        glass_mode: true,
    },
    LamzuModel {
        pid: 0x0050,
        name: "LAMZU MAYA V2",
        model: "maya-v2",
        row: "Maya(LM20)",
        sensor_offset: true,
        adaptive_dpi: true,
        glass_mode: true,
    },
    LamzuModel {
        pid: 0x0052,
        name: "LAMZU MAYA M",
        model: "maya-m",
        row: "MAYA M(LM20)",
        sensor_offset: true,
        adaptive_dpi: true,
        glass_mode: true,
    },
    LamzuModel {
        pid: 0x0056,
        name: "LAMZU ORCUS V2",
        model: "orcus-v2",
        row: "Orcus V2(LM20)",
        sensor_offset: false,
        adaptive_dpi: false,
        glass_mode: false,
    },
    LamzuModel {
        pid: 0x0046,
        name: "LAMZU ATLANTIS MINI V2",
        model: "atlantis-mini-v2",
        row: "Mini(LM20)",
        sensor_offset: true,
        adaptive_dpi: true,
        glass_mode: true,
    },
];

pub fn model_by_pid(pid: u16) -> Option<&'static LamzuModel> {
    MODELS.iter().find(|m| m.pid == pid)
}

/// センサー型番コマンド (cmd 143) の応答 → (表示名, DPI 上限, DPI 刻み, LOD 候補 (mm×10))。
/// 公式 `T2()`: 1=PAW3395 (26000, 1/2mm)、2=PAW3950 (30000, 0.7/1/2mm)、
/// 4=PAW3955 (50000, 0.7/1/1.3/1.5/1.7mm、1 刻み)。不明は機種定義の既定 (50000, 0.7/1/2)。
pub fn sensor_profile(code: u8) -> (&'static str, u32, u32, &'static [i64]) {
    match code {
        1 => ("PAW3395", 26000, 50, &[10, 20]),
        2 => ("PAW3950", 30000, 50, &[7, 10, 20]),
        4 => ("PAW3955", 50000, 1, &[7, 10, 13, 15, 17]),
        _ => ("", 50000, 50, &[7, 10, 20]),
    }
}

/// 公式の速度連動 DPI プリセット (設定画面 `AdaDPI*PointList`)。
pub fn adaptive_presets() -> Vec<(String, Vec<AdaptivePoint>)> {
    let p = |v: &[(u16, f32)]| {
        v.iter()
            .map(|&(speed, ratio)| AdaptivePoint { speed, ratio })
            .collect::<Vec<_>>()
    };
    vec![
        ("クラシック".into(), p(&[(0, 1.0), (20, 1.0), (40, 1.0)])),
        ("ナチュラル".into(), p(&[(0, 1.5), (20, 0.75), (40, 0.1)])),
        ("リープ".into(), p(&[(0, 0.1), (20, 0.75), (40, 1.5)])),
    ]
}

fn opts(v: &[(i64, &str)]) -> Vec<ChoiceOpt> {
    v.iter().map(|(a, b)| ChoiceOpt::new(*a, *b)).collect()
}

/// 機種・接続・センサーに合わせた Capabilities。
pub fn caps(model: &LamzuModel, transport: Transport, sensor: u8) -> Capabilities {
    use section::*;
    let wireless = transport == Transport::Wireless24G;
    let (_, dpi_max, dpi_step, lods) = sensor_profile(sensor);
    let polling: &[(i64, &str)] = if wireless {
        // 公式: 無線は PollingRateWireless = 1000;2000;4000;8000。
        &[(1000, "1K"), (2000, "2K"), (4000, "4K"), (8000, "8K")]
    } else {
        &[(500, "500"), (1000, "1K"), (2000, "2K"), (4000, "4K"), (8000, "8K")]
    };
    let lod_opts: Vec<ChoiceOpt> = lods
        .iter()
        .map(|&v| {
            let label = if v % 10 == 0 {
                format!("{}mm", v / 10)
            } else {
                format!("{:.1}mm", v as f32 / 10.)
            };
            ChoiceOpt::new(v, &label)
        })
        .collect();

    let mut settings = vec![
        SettingSpec::choice("polling_wireless", PERFORMANCE, "ポーリングレート", opts(polling))
            .help("マウスのデータ更新頻度 (Hz)"),
        SettingSpec::toggle("hyper_mode", PERFORMANCE, "競技モード").help(
            "OFF 時はハイスピードモード。最高の性能と電池持ちを両立するよう調整されたモード",
        ),
        SettingSpec::toggle("tracking_mode", PERFORMANCE, "20K FPS")
            .help("20kHz スキャンの極限性能モード。競技モードより電力消費が大きい")
            .enabled_when(
                "hyper_mode",
                SettingValue::Bool(true),
                "先に競技モードを ON にしてください",
            )
            .confirm(
                "20K FPS は競技モードに比べて電池の持ちが大幅に短くなります。切り替えますか？",
            ),
        SettingSpec::choice("lod", SENSOR, "LOD", lod_opts)
            .help("マウスがトラッキング面から離れ、センサーが反応しなくなる高さ"),
    ];
    if model.glass_mode {
        settings.push(
            SettingSpec::toggle("glass_mode", SENSOR, "ガラスモード")
                .help("ガラス製マウスパッドを自動検知・適応し、LOD 調整を無効にします"),
        );
        // ガラスモード中は LOD を触らせない (公式と同じ)。
        if let Some(l) = settings.iter_mut().find(|s| s.id == "lod") {
            l.enabled_when = Some((
                "glass_mode".into(),
                SettingValue::Bool(false),
                "ガラスモード中は LOD を変更できません".into(),
            ));
        }
    }
    settings.extend([
        SettingSpec::toggle("motion_sync", SENSOR, "移動同期")
            .help("PC の処理にマウスの動きを同期させ、軌跡をより正確にします"),
        SettingSpec::toggle("angle_snap", SENSOR, "直線補正")
            .help("マウス入力をより直線的で正確にします"),
        SettingSpec::toggle("ripple", SENSOR, "リップル")
            .help("高速なマウス動作中にポインタの波状の揺れを排除します"),
        SettingSpec::slider("angle_tune", SENSOR, "角度調整", -30., 30., 1., "°")
            .help("グリップに合わせてセンサー角度を補正し、斜めに持っても水平に動かせます"),
    ]);
    if model.sensor_offset {
        settings.push(
            SettingSpec::slider("sensor_offset", SENSOR, "センサー中心オフセット", -5., 5., 1., "")
                .help("センサーの中心位置の補正 (公式 SensorCenterOffset)"),
        );
    }
    settings.extend([
        SettingSpec::slider("debounce", CLICK, "デバウンスタイム", 0., 15., 1., "ms").help(
            "低すぎるとダブルクリックが起きることがあります。ゲーム向けの推奨は 2–4ms",
        ),
        SettingSpec::toggle("rapid_left", CLICK, "高速トリガー (左クリック)")
            .help("アルゴリズムでキー応答を最速化 — 誤ってダブルクリックする可能性があります"),
        SettingSpec::toggle("rapid_right", CLICK, "高速トリガー (右クリック)")
            .help("アルゴリズムでキー応答を最速化 — 誤ってダブルクリックする可能性があります"),
        SettingSpec::choice(
            "bhop_dir",
            CLICK,
            "スクロールBhop",
            opts(&[(0, "オフ"), (2, "スクロールアップ"), (3, "スクロールダウン")]),
        )
        .help("最初のスクロール信号を無効化し、応答時間内に 2 回目が無ければスクロールしません"),
        SettingSpec::choice(
            "bhop_ms",
            CLICK,
            "ホイール応答時間",
            opts(&[
                (200, "200ms"),
                (400, "400ms"),
                (600, "600ms"),
                (800, "800ms"),
                (1000, "1000ms"),
            ]),
        )
        .help("スクロールBhop で 2 回目のスクロールを待つ時間"),
        SettingSpec::choice(
            "sleep_min",
            POWER,
            "スリープ時間",
            opts(&[
                (1, "1分"),
                (5, "5分"),
                (10, "10分"),
                (15, "15分"),
                (20, "20分"),
                (25, "25分"),
            ]),
        )
        .help("一定時間無操作だとスリープします"),
    ]);
    if wireless {
        settings.push(
            SettingSpec::toggle("dongle_led", LIGHTING, "ドングルLED")
                .help("レシーバーのライトでマウスの状態を表示"),
        );
        for (id, name) in [
            ("dongle_led1", "LED 1"),
            ("dongle_led2", "LED 2"),
            ("dongle_led3", "LED 3"),
        ] {
            settings.push(
                SettingSpec::choice(
                    id,
                    LIGHTING,
                    name,
                    opts(&[(6, "バッテリー"), (11, "DPI"), (10, "ポーリングレート"), (12, "信号")]),
                )
                .when("dongle_led", SettingValue::Bool(true)),
            );
        }
    }
    Capabilities {
        settings,
        actions: vec![
            ActionSpec::new("reset_profile", section::SYSTEM, "プロファイルをリセット")
                .help("現在のプロファイルだけを工場出荷時の値に戻します")
                .danger("現在のプロファイルの設定がすべて初期化されます"),
            ActionSpec::new("factory_reset", section::SYSTEM, "完全初期化")
                .help(if wireless {
                    "マウスとレシーバーを工場出荷時のデフォルト設定に戻します"
                } else {
                    "マウスを工場出荷時のデフォルト設定に戻します"
                })
                .danger("OK を押すと、マウスは工場出荷時のデフォルト設定に戻ります"),
        ],
        profiles: Some(ProfileCaps {
            count: 4,
            renamable: false,
        }),
        mouse: Some(MouseCaps {
            dpi_min: 50,
            commit_dpi_on_release: false,
            dpi_max,
            dpi_step,
            min_stages: 1,
            max_stages: 5,
            xy_split: true,
            stage_colors: true,
            stage_toggle: false,
            buttons: ["左クリック", "右クリック", "ホイールクリック", "戻る", "進む"]
                .iter()
                .map(|n| ButtonSlot { name: (*n).into() })
                .collect(),
            button_kinds: vec![
                ButtonActionKind::Mouse,
                ButtonActionKind::Key,
                ButtonActionKind::Media,
                ButtonActionKind::Dpi,
                ButtonActionKind::Fire,
                ButtonActionKind::PollingCycle,
                ButtonActionKind::ProfileCycle,
                ButtonActionKind::Disabled,
            ],
            button_options: None,
            require_left_click: true,
            macros: None,
            live_buttons: vec![],
            adaptive_dpi: model.adaptive_dpi.then(|| AdaptiveCaps {
                min_points: 2,
                max_points: 5,
                speed_max: 105,
                ratio_max: 1.5,
                presets: adaptive_presets(),
            }),
        }),
        keyboard: None,
        lighting: vec![],
        battery: true,
        peq: None,
        firmware_feed: None,
    }
}

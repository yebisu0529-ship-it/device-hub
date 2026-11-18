//! オーディオ: USB DAC のパラメトリック EQ (peq.szwalkplay.com の上位互換)。
//!
//! - 応答グラフ: 合成カーブ + 選択バンド単体のカーブ。番号付きのハンドルを
//!   **ドラッグで周波数/ゲインを同時に**動かし、ホイールで Q を変える。
//!   ホバー中はカーソル位置の周波数と合成ゲインを表示。
//! - バンド一覧 (公式の縦列表示に相当) → 選択バンドの詳細編集。
//! - プリアンプ (EQ ボリューム) + クリップ警告 + 自動プリアンプ。
//! - プリセット: 内蔵 (DEF/低音/V字/ボーカル/FPS 足音 …) + 保存したもの (prefs)。
//!   AutoEQ / Equalizer APO 形式 (ParametricEQ.txt) の貼り付け・コピー。
//!
//! `Capabilities::peq` があるときだけ EQ UI を出す。残りの AUDIO 設定は
//! generic のカード (eq_volume と eq_def は EQ カード側へ移している)。

use crate::app::{CbXY, PopItem, Shell, Tab};
use crate::prefs::EqPreset;
use crate::ui::{self, Btn};
use crate::{motion, theme};
use devhub_core::model::{PeqBand, PeqCaps, PeqFilter};
use devhub_core::*;
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

/// グラフの周波数レンジ (対数軸)。
const F_LO: f64 = 20.;
const F_HI: f64 = 20_000.;
/// グラフの高さ。
const GRAPH_H: f32 = 236.;
/// カーブのサンプル数。
const N_PTS: usize = 180;
/// EQ ボリューム (プリアンプ) の設定 id。
const PREAMP_ID: &str = "eq_volume";
/// 空きバンドに置く既定の周波数 (10 バンド・オクターブ間隔)。
const DEFAULT_FREQS: [u16; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

/// freq (Hz) → 0..1 (対数)。
fn fx(f: f64) -> f32 {
    ((f.clamp(F_LO, F_HI).ln() - F_LO.ln()) / (F_HI.ln() - F_LO.ln())) as f32
}
/// 0..1 (対数) → freq (Hz)。
fn un_fx(t: f64) -> f64 {
    (F_LO.ln() + t.clamp(0., 1.) * (F_HI.ln() - F_LO.ln())).exp()
}

/// バンドごとの色 (グラフのハンドルと一覧で共通)。
fn band_color(i: usize) -> Hsla {
    const H: [f32; 10] = [0.58, 0.50, 0.40, 0.30, 0.15, 0.08, 0.0, 0.92, 0.80, 0.70];
    hsla(H[i % H.len()], 0.72, 0.64, 1.)
}

fn fmt_freq(f: u16) -> String {
    if f >= 10_000 {
        format!("{}k", (f as f32 / 1000.).round() as u32)
    } else if f >= 1000 {
        let k = f as f32 / 1000.;
        if (k * 10.).round() % 10. == 0. {
            format!("{}k", k.round() as u32)
        } else {
            format!("{:.1}k", k)
        }
    } else {
        format!("{f}")
    }
}

fn filter_name(f: PeqFilter) -> &'static str {
    match f {
        PeqFilter::LowShelf => "ローシェルフ",
        PeqFilter::Peak => "ピーキング",
        PeqFilter::HighShelf => "ハイシェルフ",
        PeqFilter::LowPass => "ローパス",
        PeqFilter::HighPass => "ハイパス",
    }
}

/// ゲインを使わないフィルター (縦ドラッグを無視する)。
fn gainless(f: PeqFilter) -> bool {
    matches!(f, PeqFilter::LowPass | PeqFilter::HighPass)
}

/// ドラッグ時の周波数の丸め (細かすぎる値を作らない)。
fn round_freq(f: f64) -> f64 {
    if f >= 1000. {
        (f / 10.).round() * 10.
    } else {
        f.round()
    }
}

/// 合成カーブの最大値 (dB)。クリップ判定・自動プリアンプ用。
fn peak_db(bands: &[PeqBand]) -> f64 {
    (0..=N_PTS)
        .map(|i| devhub_core::peq::response_db(bands, un_fx(i as f64 / N_PTS as f64)))
        .fold(f64::MIN, f64::max)
}

fn same_bands(a: &[PeqBand], b: &[PeqBand]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.freq == y.freq
                && x.filter == y.filter
                && (x.gain - y.gain).abs() < 0.05
                && (x.filter == PeqFilter::LowShelf || (x.q - y.q).abs() < 0.01)
        })
}

// ---------------------------------------------------------------- プリセット

/// (フィルター, 周波数, Q, ゲイン) の列をバンド数に合わせて埋める。
fn fill_bands(n: usize, list: &[(PeqFilter, u16, f32, f32)], pc: &PeqCaps) -> Vec<PeqBand> {
    let mut out: Vec<PeqBand> = list
        .iter()
        .take(n)
        .map(|&(filter, freq, q, gain)| PeqBand {
            freq: freq.clamp(pc.freq_min, pc.freq_max),
            q: q.clamp(pc.q_min, pc.q_max),
            gain: gain.clamp(pc.gain_min, pc.gain_max),
            filter: if pc.filters.contains(&filter) {
                filter
            } else {
                PeqFilter::Peak
            },
        })
        .collect();
    // 空きバンドは既定の周波数のうち使われていないものを 0dB で置く。
    let used: Vec<u16> = out.iter().map(|b| b.freq).collect();
    let mut spare = DEFAULT_FREQS
        .iter()
        .copied()
        .filter(move |f| !used.contains(f));
    while out.len() < n {
        let f = spare.next().unwrap_or(1000);
        out.push(PeqBand {
            freq: f.clamp(pc.freq_min, pc.freq_max),
            q: 0.75_f32.clamp(pc.q_min, pc.q_max),
            gain: 0.,
            filter: PeqFilter::Peak,
        });
    }
    out.sort_by_key(|b| b.freq);
    out
}

type PresetDef = (
    &'static str,
    &'static str,
    Vec<(PeqFilter, u16, f32, f32)>,
    f32,
);

/// 内蔵プリセット: (名前, 説明, バンド列, プリアンプ)。
fn builtin_presets() -> Vec<PresetDef> {
    use PeqFilter::*;
    vec![
        (
            "フラット",
            "全バンド 0dB (既定の周波数)",
            DEFAULT_FREQS.iter().map(|&f| (Peak, f, 0.75, 0.)).collect(),
            0.,
        ),
        (
            "DEF (公式)",
            "公式アプリの DEF と同じ 8 バンド",
            [50, 200, 500, 1000, 2500, 5000, 10000, 15000]
                .iter()
                .map(|&f| (Peak, f, 0.75, 0.))
                .collect(),
            0.,
        ),
        (
            "低音ブースト",
            "100Hz 以下を持ち上げる",
            vec![(LowShelf, 105, 0.7, 6.), (Peak, 300, 1.0, -1.)],
            -6.,
        ),
        (
            "V 字 (ドンシャリ)",
            "低域と高域を上げ、中域を少し下げる",
            vec![
                (LowShelf, 100, 0.7, 4.),
                (Peak, 1000, 0.8, -2.5),
                (HighShelf, 8000, 0.7, 4.),
            ],
            -5.,
        ),
        (
            "ボーカル",
            "声の帯域を前に出す",
            vec![
                (Peak, 200, 1.0, -2.),
                (Peak, 1500, 1.0, 2.),
                (Peak, 3000, 1.2, 2.5),
                (HighShelf, 10000, 0.7, -1.),
            ],
            -3.,
        ),
        (
            "FPS 足音",
            "爆発音などの低域を抑え、足音の 2〜5kHz を強調",
            vec![
                (LowShelf, 120, 0.7, -5.),
                (Peak, 400, 1.0, -2.),
                (Peak, 2500, 1.2, 4.),
                (Peak, 5000, 1.5, 3.),
                (HighShelf, 10000, 0.7, -2.),
            ],
            -4.,
        ),
        (
            "ハーマン風",
            "IEM 向けハーマンターゲットに近い傾き",
            vec![
                (LowShelf, 105, 0.7, 4.5),
                (Peak, 3000, 1.4, 1.5),
                (HighShelf, 10000, 0.7, -2.),
            ],
            -5.,
        ),
        (
            "高域を抑える",
            "刺さる高音を和らげる",
            vec![(HighShelf, 6000, 0.7, -3.), (Peak, 8000, 2.0, -2.)],
            0.,
        ),
    ]
}

// ---------------------------------------------------------------- AutoEQ (Equalizer APO) 形式

/// `Preamp: -6.2 dB` / `Filter 1: ON PK Fc 105 Hz Gain -3.4 dB Q 0.70` を読む。
/// 戻り値: (プリアンプ, フィルター列, 無視した行数)。
fn parse_apo(text: &str) -> (Option<f32>, Vec<(PeqFilter, u16, f32, f32)>, usize) {
    let mut preamp = None;
    let mut out = vec![];
    let mut skipped = 0;
    for line in text.lines() {
        let line = line.trim();
        let low = line.to_ascii_lowercase();
        if low.starts_with("preamp") {
            preamp = line.split_whitespace().find_map(|t| {
                t.trim_end_matches("dB")
                    .replace(',', ".")
                    .parse::<f32>()
                    .ok()
            });
            continue;
        }
        if !low.starts_with("filter") {
            continue;
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        let Some(on) = toks.iter().position(|t| t.eq_ignore_ascii_case("ON")) else {
            skipped += 1;
            continue;
        };
        let Some(kind) = toks.get(on + 1) else {
            skipped += 1;
            continue;
        };
        let filter = match kind.to_ascii_uppercase().as_str() {
            "PK" | "PEQ" | "MODAL" => PeqFilter::Peak,
            "LS" | "LSC" | "LSQ" => PeqFilter::LowShelf,
            "HS" | "HSC" | "HSQ" => PeqFilter::HighShelf,
            "LP" | "LPQ" => PeqFilter::LowPass,
            "HP" | "HPQ" => PeqFilter::HighPass,
            _ => {
                skipped += 1;
                continue;
            }
        };
        let val = |key: &str| -> Option<f32> {
            toks.iter()
                .position(|t| t.eq_ignore_ascii_case(key))
                .and_then(|i| toks.get(i + 1))
                .and_then(|v| v.replace(',', ".").parse::<f32>().ok())
        };
        let Some(fc) = val("Fc") else {
            skipped += 1;
            continue;
        };
        out.push((
            filter,
            fc.round().clamp(1., 65_535.) as u16,
            val("Q").unwrap_or(0.71),
            val("Gain").unwrap_or(0.),
        ));
    }
    (preamp, out, skipped)
}

fn to_apo(bands: &[PeqBand], preamp: Option<f32>) -> String {
    let mut s = String::new();
    if let Some(p) = preamp {
        s.push_str(&format!("Preamp: {p:.1} dB\n"));
    }
    for (i, b) in bands.iter().enumerate() {
        let t = match b.filter {
            PeqFilter::Peak => "PK",
            PeqFilter::LowShelf => "LSC",
            PeqFilter::HighShelf => "HSC",
            PeqFilter::LowPass => "LPQ",
            PeqFilter::HighPass => "HPQ",
        };
        s.push_str(&format!(
            "Filter {}: ON {t} Fc {} Hz Gain {:.1} dB Q {:.2}\n",
            i + 1,
            b.freq,
            b.gain,
            b.q
        ));
    }
    s
}

/// 選択バンドの現在値 (ドラッグ/ホイール中に古い値から計算しないため)。
fn live_band(this: &Shell, i: usize) -> Option<PeqBand> {
    this.cur()
        .and_then(|s| s.state.as_ref())
        .and_then(|s| s.peq.as_ref())
        .and_then(|v| v.get(i).copied())
}

impl Shell {
    pub fn page_audio(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sess = self.cur();
        let caps = sess.and_then(|s| s.caps.clone());
        let state = sess.and_then(|s| s.state.clone());
        let mut body = div().flex().flex_col().gap(px(theme::GAP));
        if let (Some(caps), Some(state)) = (caps, state) {
            if let (Some(pc), Some(bands)) = (caps.peq.clone(), state.peq.clone()) {
                if self.peq_sel >= bands.len() {
                    self.peq_sel = 0;
                }
                body = body.child(self.peq_card(&caps, &state, &pc, &bands, window, cx));
            }
        }
        let rest = self.section_cards(Tab::Audio, window, cx);
        if !rest.is_empty() {
            body = body.child(ui::grid2(rest));
        }
        body
    }

    /// 現在のプリアンプ値と範囲 (eq_volume が無い機種は None)。
    fn preamp(caps: &Capabilities, state: &DeviceState) -> Option<(f32, f32, f32, f32)> {
        let sp = caps.setting(PREAMP_ID)?;
        let SettingKind::Slider { min, max, step, .. } = sp.kind.clone() else {
            return None;
        };
        let v = match state.settings.get(PREAMP_ID) {
            Some(SettingValue::Num(v)) => *v,
            _ => 0.,
        };
        Some((v, min, max, step))
    }

    fn set_preamp(&mut self, v: f32, cx: &mut Context<Self>) {
        self.apply(
            Change::Setting {
                id: PREAMP_ID.into(),
                value: SettingValue::Num(v),
            },
            cx,
        );
    }

    /// プリセットを当てる (バンド + プリアンプ)。
    fn apply_eq(&mut self, bands: Vec<PeqBand>, preamp: Option<f32>, cx: &mut Context<Self>) {
        let (caps, state) = match self.cur() {
            Some(s) => (s.caps.clone(), s.state.clone()),
            None => return,
        };
        if state.as_ref().and_then(|s| s.peq.as_ref()) != Some(&bands) {
            self.apply(Change::PeqAll { bands }, cx);
        }
        if let (Some(p), Some(caps), Some(state)) = (preamp, caps, state) {
            if let Some((cur, min, max, step)) = Self::preamp(&caps, &state) {
                let v = ui::quantize(p.floor(), min, max, step);
                if (v - cur).abs() > 0.01 {
                    self.set_preamp(v, cx);
                }
            }
        }
    }

    /// EQ カード全体。
    fn peq_card(
        &mut self,
        caps: &Capabilities,
        state: &DeviceState,
        pc: &PeqCaps,
        bands: &[PeqBand],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let trailing = self.peq_toolbar(caps, state, pc, bands, cx);
        let graph = self.peq_graph(pc, bands, cx);
        let preamp = self.peq_preamp_row(caps, state, bands, window, cx);
        let strip = self.peq_strip(bands, cx);
        let editor = self.peq_editor(pc, bands, window, cx);
        let mut body = ui::body().pt(px(8.)).gap(px(12.)).child(graph);
        if let Some(p) = preamp {
            body = body.child(p);
        }
        ui::card_titled(
            "パラメトリック EQ",
            "audio-lines",
            Some(trailing.into_any_element()),
        )
        .child(body.child(strip).child(editor))
        .into_any_element()
    }

    // ------------------------------------------------------------ ツールバー (プリセット)

    fn peq_toolbar(
        &mut self,
        caps: &Capabilities,
        state: &DeviceState,
        pc: &PeqCaps,
        bands: &[PeqBand],
        cx: &mut Context<Self>,
    ) -> Div {
        let n = pc.bands as usize;
        let builtins: Vec<(String, String, Vec<PeqBand>, f32)> = builtin_presets()
            .into_iter()
            .map(|(name, sub, list, pre)| {
                (
                    name.to_string(),
                    sub.to_string(),
                    fill_bands(n, &list, pc),
                    pre,
                )
            })
            .collect();
        let users: Vec<EqPreset> = self.prefs.eq_presets.clone();
        // 今の EQ に一致するプリセット
        let cur_builtin = builtins.iter().position(|b| same_bands(&b.2, bands));
        let cur_user = users.iter().position(|u| same_bands(&u.bands, bands));
        let cur_label = match (cur_user, cur_builtin) {
            (Some(i), _) => users[i].name.clone(),
            (None, Some(i)) => builtins[i].0.clone(),
            _ => "カスタム".to_string(),
        };
        let mut items: Vec<PopItem> = builtins
            .iter()
            .enumerate()
            .map(|(i, b)| PopItem {
                icon: Some("audio-lines"),
                ..PopItem::new(b.0.clone(), cur_user.is_none() && cur_builtin == Some(i))
            })
            .collect();
        for (i, u) in users.iter().enumerate() {
            items.push(PopItem {
                sub: Some("保存済み".into()),
                icon: Some("bookmark"),
                ..PopItem::new(u.name.clone(), cur_user == Some(i))
            });
        }
        let nb = builtins.len();
        let (b2, u2, pc2) = (builtins.clone(), users.clone(), pc.clone());
        let pick = ui::select(
            "peq-preset",
            items,
            &cur_label,
            190.,
            cx,
            move |this, i, _, cx| {
                if i < nb {
                    // プリアンプはカーブのピークから自動 (クリップしない最大値)。
                    let (_, _, bands, _) = b2[i].clone();
                    let pre = -(peak_db(&bands).max(0.)).ceil() as f32;
                    this.apply_eq(bands, Some(pre), cx);
                } else if let Some(u) = u2.get(i - nb) {
                    let list: Vec<_> = u
                        .bands
                        .iter()
                        .map(|b| (b.filter, b.freq, b.q, b.gain))
                        .collect();
                    let bands = fill_bands(pc2.bands as usize, &list, &pc2);
                    this.apply_eq(bands, u.preamp, cx);
                }
            },
        );
        let preamp_now = Self::preamp(caps, state).map(|p| p.0);
        let mut bar = div().flex().items_center().gap(px(6.)).child(pick);
        // 保存
        {
            let cur = bands.to_vec();
            bar = bar.child(ui::icon_button(
                "peq-save",
                "save-plus",
                cx,
                move |this, w, cx| {
                    let cur = cur.clone();
                    this.modal = Some(crate::app::Modal::Text {
                        title: "EQ プリセットの名前".into(),
                        on_ok: Rc::new(move |this, v, _, cx| {
                            let name = v.trim().to_string();
                            if name.is_empty() {
                                return;
                            }
                            let p = EqPreset {
                                name: name.clone(),
                                preamp: preamp_now,
                                bands: cur.clone(),
                            };
                            // 同名は上書き
                            if let Some(e) =
                                this.prefs.eq_presets.iter_mut().find(|e| e.name == name)
                            {
                                *e = p;
                            } else {
                                this.prefs.eq_presets.push(p);
                            }
                            this.prefs.save();
                            this.toast(format!("「{name}」を保存しました"), false, cx);
                            cx.notify();
                        }),
                    });
                    this.input("modal", "", "", w, cx);
                    this.set_input("modal", "マイ EQ", w, cx);
                    cx.notify();
                },
            ));
        }
        // 削除 (保存したプリセットと一致しているとき)
        if let Some(i) = cur_user {
            let name = users[i].name.clone();
            bar = bar.child(ui::icon_button(
                "peq-del",
                "trash",
                cx,
                move |this, _, cx| {
                    let name = name.clone();
                    let n2 = name.clone();
                    this.confirm(
                        "プリセットを削除",
                        &format!("「{name}」を削除しますか? (デバイスの EQ は変わりません)"),
                        "削除",
                        true,
                        Rc::new(move |this, _, cx| {
                            this.prefs.eq_presets.retain(|e| e.name != n2);
                            this.prefs.save();
                            cx.notify();
                        }),
                        cx,
                    );
                },
            ));
        }
        // AutoEQ 貼り付け
        bar = bar.child(ui::button(
            "peq-paste",
            "AutoEQ 貼り付け",
            Some("clipboard-paste"),
            Btn::Glass,
            cx,
            move |this, _, cx| this.peq_paste(cx),
        ));
        // コピー
        {
            let text = to_apo(bands, preamp_now);
            bar = bar.child(ui::icon_button(
                "peq-copy",
                "clipboard-copy",
                cx,
                move |this, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    this.toast("EQ を Equalizer APO 形式でコピーしました", false, cx);
                },
            ));
        }
        bar
    }

    /// クリップボードの AutoEQ / Equalizer APO テキストを当てる。
    fn peq_paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            self.toast("クリップボードにテキストがありません", true, cx);
            return;
        };
        let Some(pc) = self
            .cur()
            .and_then(|s| s.caps.as_ref())
            .and_then(|c| c.peq.clone())
        else {
            return;
        };
        let (preamp, list, skipped) = parse_apo(&text);
        if list.is_empty() {
            self.toast(
                "AutoEQ (ParametricEQ.txt) 形式の行が見つかりません",
                true,
                cx,
            );
            return;
        }
        let n = pc.bands as usize;
        let over = list.len().saturating_sub(n);
        let bands = fill_bands(n, &list, &pc);
        // プリアンプが無いテキストは、クリップしない値を自動で付ける。
        let pre = preamp.or_else(|| Some(-(peak_db(&bands).max(0.)).ceil() as f32));
        self.apply_eq(bands, pre, cx);
        let mut msg = format!("{} バンドを読み込みました", list.len().min(n));
        if over > 0 {
            msg.push_str(&format!(" (バンド数を超えた {over} 個は無視)"));
        }
        if skipped > 0 {
            msg.push_str(&format!(" · 未対応の行 {skipped}"));
        }
        self.toast(msg, over > 0, cx);
    }

    // ------------------------------------------------------------ グラフ

    fn peq_graph(&mut self, pc: &PeqCaps, bands: &[PeqBand], cx: &mut Context<Self>) -> Div {
        let p = theme::pal();
        let sel = self.peq_sel;
        let span = (pc.gain_max.abs().max(pc.gain_min.abs()) as f64 + 3.).max(6.);
        let total: Vec<f32> = (0..=N_PTS)
            .map(|i| devhub_core::peq::response_db(bands, un_fx(i as f64 / N_PTS as f64)) as f32)
            .collect();
        let single: Vec<f32> = match bands.get(sel) {
            Some(b) => (0..=N_PTS)
                .map(|i| {
                    devhub_core::peq::response_db(
                        std::slice::from_ref(b),
                        un_fx(i as f64 / N_PTS as f64),
                    ) as f32
                })
                .collect(),
            None => vec![],
        };
        let grid = theme::alpha(p.faint, 0.12);
        let grid_major = theme::alpha(p.faint, 0.26);
        let zero = theme::alpha(p.faint, 0.5);
        let accent = p.accent;
        let sel_col = band_color(sel);
        let spanf = span as f32;
        let db_lines: Vec<f32> = {
            let stepd = if span > 12. { 6. } else { 3. };
            let mut v = vec![];
            let mut d = -((span / stepd).floor() * stepd);
            while d <= span + 0.01 {
                v.push(d as f32);
                d += stepd;
            }
            v
        };
        let db_lines2 = db_lines.clone();
        let cv = canvas(
            move |_, _, _| (total.clone(), single.clone(), db_lines2.clone()),
            move |b, (total, single, dbl), w, _| {
                let (wd, h) = (f32::from(b.size.width), f32::from(b.size.height));
                let y_at = |db: f32| px(h * (1. - ((db / spanf).clamp(-1., 1.) + 1.) * 0.5));
                let x_i = |i: usize| px(wd * i as f32 / N_PTS as f32);
                // 縦グリッド (1-2-5 系列)
                for f in [
                    30., 40., 50., 60., 70., 80., 90., 100., 200., 300., 400., 500., 600., 700.,
                    800., 900., 1000., 2000., 3000., 4000., 5000., 6000., 7000., 8000., 9000.,
                    10000.,
                ] {
                    let major = f == 100. || f == 1000. || f == 10000.;
                    let x = px(wd * fx(f));
                    let mut pb = PathBuilder::stroke(px(1.));
                    pb.move_to(b.origin + point(x, px(0.)));
                    pb.line_to(b.origin + point(x, px(h)));
                    if let Ok(path) = pb.build() {
                        w.paint_path(path, if major { grid_major } else { grid });
                    }
                }
                for db in dbl {
                    let y = y_at(db);
                    let mut pb = PathBuilder::stroke(px(1.));
                    pb.move_to(b.origin + point(px(0.), y));
                    pb.line_to(b.origin + point(px(wd), y));
                    if let Ok(path) = pb.build() {
                        w.paint_path(path, if db == 0. { zero } else { grid });
                    }
                }
                // 選択バンド単体 (塗り + 線)
                if !single.is_empty() {
                    let mut pf = PathBuilder::fill();
                    pf.move_to(b.origin + point(px(0.), y_at(0.)));
                    for (i, db) in single.iter().enumerate() {
                        pf.line_to(b.origin + point(x_i(i), y_at(*db)));
                    }
                    pf.line_to(b.origin + point(px(wd), y_at(0.)));
                    pf.close();
                    if let Ok(path) = pf.build() {
                        w.paint_path(path, theme::alpha(sel_col, 0.16));
                    }
                    let mut pb = PathBuilder::stroke(px(1.2));
                    for (i, db) in single.iter().enumerate() {
                        let pt = b.origin + point(x_i(i), y_at(*db));
                        if i == 0 {
                            pb.move_to(pt)
                        } else {
                            pb.line_to(pt)
                        }
                    }
                    if let Ok(path) = pb.build() {
                        w.paint_path(path, theme::alpha(sel_col, 0.7));
                    }
                }
                // 合成カーブ (薄い塗り + 太線)
                let mut pf = PathBuilder::fill();
                pf.move_to(b.origin + point(px(0.), y_at(0.)));
                for (i, db) in total.iter().enumerate() {
                    pf.line_to(b.origin + point(x_i(i), y_at(*db)));
                }
                pf.line_to(b.origin + point(px(wd), y_at(0.)));
                pf.close();
                if let Ok(path) = pf.build() {
                    w.paint_path(path, theme::alpha(accent, 0.10));
                }
                let mut pb = PathBuilder::stroke(px(2.2));
                for (i, db) in total.iter().enumerate() {
                    let pt = b.origin + point(x_i(i), y_at(*db));
                    if i == 0 {
                        pb.move_to(pt)
                    } else {
                        pb.line_to(pt)
                    }
                }
                if let Ok(path) = pb.build() {
                    w.paint_path(path, accent);
                }
            },
        )
        .absolute()
        .inset_0();

        // ハンドル (番号入りの丸)。div なので文字が描ける。
        let mut plot = div()
            .id("peq-plot")
            .relative()
            .flex_1()
            .h(px(GRAPH_H))
            .rounded(px(10.))
            .bg(theme::alpha(p.faint, 0.05))
            .border_1()
            .border_color(p.hair)
            .overflow_hidden()
            .cursor(CursorStyle::Crosshair)
            .child(cv)
            .child(motion::track("peq-graph"));
        // 選択中は最後に描いて最前面へ
        let mut order: Vec<usize> = (0..bands.len()).filter(|i| *i != sel).collect();
        if sel < bands.len() {
            order.push(sel);
        }
        for i in order {
            let b = bands[i];
            let on = i == sel;
            let r = if on { 11. } else { 9. };
            let col = band_color(i);
            let y = if gainless(b.filter) {
                0.
            } else {
                b.gain as f64
            };
            let ty = 1. - ((y / span).clamp(-1., 1.) + 1.) * 0.5;
            plot = plot.child(
                div()
                    .absolute()
                    .left(relative(fx(b.freq as f64)))
                    .top(relative(ty as f32))
                    .ml(px(-r))
                    .mt(px(-r))
                    .size(px(r * 2.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if on { col } else { theme::alpha(col, 0.25) })
                    .border_2()
                    .border_color(if on { theme::hex(0xffffff, 0.92) } else { col })
                    .text_size(px(if on { 11. } else { 10. }))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if on { theme::hex(0x0b1020, 1.) } else { p.fg })
                    .child(format!("{}", i + 1)),
            );
        }
        // ホバー中のカーソル読み取り
        if let Some(r) = motion::rect("peq-graph") {
            let [mx, my] = motion::pointer();
            if self.drag.is_none()
                && mx >= r[0]
                && mx <= r[0] + r[2]
                && my >= r[1]
                && my <= r[1] + r[3]
            {
                let f = un_fx(((mx - r[0]) / r[2].max(1.)) as f64);
                let db = devhub_core::peq::response_db(bands, f);
                plot = plot
                    .child(
                        div()
                            .absolute()
                            .left(px(mx - r[0]))
                            .top_0()
                            .bottom_0()
                            .w(px(1.))
                            .bg(theme::alpha(p.fg, 0.16)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(8.))
                            .right(px(10.))
                            .px(px(8.))
                            .py(px(3.))
                            .rounded(px(7.))
                            .bg(theme::alpha(p.solid, 0.75))
                            .text_size(px(11.))
                            .text_color(p.sub)
                            .child(format!("{} Hz · {:+.1} dB", fmt_freq(f.round() as u16), db)),
                    );
            }
        }
        // マウス: 最寄りのハンドルを掴んで 2D ドラッグ (差分で動かすのでクリックだけでは動かない)
        let bands_v: Vec<PeqBand> = bands.to_vec();
        let pc2 = pc.clone();
        plot = plot
            .on_mouse_move(cx.listener(|_, _, _, cx| cx.notify()))
            .on_hover(cx.listener(|_, _, _, cx| cx.notify()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    motion::set_down(true);
                    let Some(r) = motion::rect("peq-graph") else {
                        return;
                    };
                    let (mx, my) = (f32::from(e.position.x), f32::from(e.position.y));
                    let tx0 = ((mx - r[0]) / r[2].max(1.)).clamp(0., 1.);
                    let ty0 = 1. - ((my - r[1]) / r[3].max(1.)).clamp(0., 1.);
                    // ピクセル距離で最寄り (ゲインレス種別は 0dB 上にいるとみなす)
                    let near = bands_v
                        .iter()
                        .enumerate()
                        .map(|(i, b)| {
                            let g = if gainless(b.filter) {
                                0.
                            } else {
                                b.gain as f64
                            };
                            let bx = fx(b.freq as f64) * r[2];
                            let by = (1. - ((g / span).clamp(-1., 1.) + 1.) * 0.5) as f32 * r[3];
                            let (dx, dy) = (bx - (mx - r[0]), by - (my - r[1]));
                            // 選択中のバンドを少し優先 (重なったとき)
                            let bias = if i == this.peq_sel { 0.6 } else { 1. };
                            (i, (dx * dx + dy * dy) * bias)
                        })
                        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                        .map(|x| x.0);
                    let Some(i) = near else { return };
                    this.peq_sel = i;
                    let orig = live_band(this, i).unwrap_or(bands_v[i]);
                    let pc = pc2.clone();
                    let (w0, h0) = (r[2], r[3]);
                    let f: CbXY = Rc::new(move |this, tx, ty, cx| {
                        if ((tx - tx0) * w0).abs() < 3. && ((ty - ty0) * h0).abs() < 3. {
                            return;
                        }
                        let mut nb = orig;
                        let nf = un_fx((fx(orig.freq as f64) + (tx - tx0)) as f64);
                        nb.freq = (round_freq(nf) as u16).clamp(pc.freq_min, pc.freq_max);
                        if !gainless(orig.filter) {
                            let g = orig.gain as f64 + (ty - ty0) as f64 * 2. * span;
                            nb.gain = ui::quantize(g as f32, pc.gain_min, pc.gain_max, 0.5);
                        }
                        if live_band(this, i) != Some(nb) {
                            this.apply(Change::Peq { band: i, value: nb }, cx);
                        }
                    });
                    this.begin_drag_xy("peq-graph", f, e.position, cx);
                    cx.notify();
                }),
            );
        // ホイール: 選択バンドの Q
        {
            let (qmin, qmax) = (pc.q_min, pc.q_max);
            plot = plot.on_scroll(cx.listener(move |this, e: &ScrollEvent, _, cx| {
                let dy = match e.delta {
                    ScrollDelta::Lines(l) => l.y,
                    ScrollDelta::Pixels(p) => f32::from(p.y) / 40.,
                };
                if dy.abs() < 0.01 {
                    return;
                }
                let i = this.peq_sel;
                let Some(mut nb) = live_band(this, i) else {
                    return;
                };
                let q = ui::quantize(nb.q * 1.12f32.powf(dy), qmin, qmax, 0.01);
                if (q - nb.q).abs() > 0.001 {
                    nb.q = q;
                    this.apply(Change::Peq { band: i, value: nb }, cx);
                }
                cx.stop_propagation();
            }));
        }

        // 軸ラベル
        let mut ylab = div().relative().w(px(30.)).h(px(GRAPH_H)).flex_none();
        for db in &db_lines {
            let ty = 1. - ((*db as f64 / span).clamp(-1., 1.) + 1.) * 0.5;
            ylab = ylab.child(
                div()
                    .absolute()
                    .right(px(6.))
                    .top(relative(ty as f32))
                    .mt(px(-7.))
                    .text_size(px(10.))
                    .text_color(p.faint)
                    .child(if *db > 0. {
                        format!("+{db}")
                    } else {
                        format!("{db}")
                    }),
            );
        }
        let mut xlab = div().relative().flex_1().h(px(14.));
        for (f, l) in [
            (20., "20"),
            (50., "50"),
            (100., "100"),
            (200., "200"),
            (500., "500"),
            (1000., "1k"),
            (2000., "2k"),
            (5000., "5k"),
            (10000., "10k"),
            (20000., "20k"),
        ] {
            let t = fx(f);
            xlab = xlab.child(
                div()
                    .absolute()
                    .left(relative(t))
                    .ml(px(if t > 0.98 {
                        -22.
                    } else if t < 0.02 {
                        0.
                    } else {
                        -10.
                    }))
                    .text_size(px(10.))
                    .text_color(p.faint)
                    .child(l),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().flex().child(ylab).child(plot))
            .child(div().flex().child(div().w(px(30.)).flex_none()).child(xlab))
            .child(
                div()
                    .pl(px(30.))
                    .pt(px(2.))
                    .text_size(px(11.))
                    .text_color(p.faint)
                    .child("ハンドルをドラッグで周波数とゲイン · ホイールで Q (帯域幅)"),
            )
    }

    // ------------------------------------------------------------ プリアンプ

    fn peq_preamp_row(
        &mut self,
        caps: &Capabilities,
        state: &DeviceState,
        bands: &[PeqBand],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let p = theme::pal();
        let (v, min, max, step) = Self::preamp(caps, state)?;
        let peak = peak_db(bands) as f32;
        let headroom = peak + v;
        let auto = ui::quantize(-(peak.max(0.)).ceil(), min, max, step);
        let inp = self.num_field(
            "peq-preamp",
            v,
            min,
            max,
            step,
            "dB",
            window,
            cx,
            Rc::new(|this, nv, cx| this.set_preamp(nv, cx)),
        );
        let status = if headroom > 0.05 {
            ui::chip(format!("クリップの可能性 +{headroom:.1}dB"), p.warn)
        } else {
            ui::chip(format!("余裕 {:.1}dB", -headroom), p.ok)
        };
        let mut right = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(status)
            .child(div().w(px(220.)).child(ui::slider(
                "peq-preamp",
                v,
                min,
                max,
                step,
                None,
                cx,
                |this, nv, cx| this.set_preamp(nv, cx),
            )))
            .child(
                div()
                    .w(px(72.))
                    .child(gpui_kit::component::input::Input::new(&inp)),
            );
        if (auto - v).abs() > 0.01 {
            right = right.child(ui::button(
                "peq-auto-pre",
                &if auto.abs() < 0.5 {
                    "自動 (0dB)".to_string()
                } else {
                    format!("自動 ({auto:+.0}dB)")
                },
                Some("wand-sparkles"),
                Btn::Glass,
                cx,
                move |this, _, cx| this.set_preamp(auto, cx),
            ));
        }
        Some(ui::row_first(
            "プリアンプ (EQ ボリューム)",
            Some("ブーストした分だけ下げると音割れしません"),
            right,
        ))
    }

    // ------------------------------------------------------------ バンド一覧

    fn peq_strip(&mut self, bands: &[PeqBand], cx: &mut Context<Self>) -> Div {
        let p = theme::pal();
        let sel = self.peq_sel;
        let mut strip = div().flex().gap(px(6.));
        for (i, b) in bands.iter().enumerate() {
            let on = i == sel;
            let col = band_color(i);
            let gain_col = if gainless(b.filter) || b.gain.abs() < 0.05 {
                p.faint
            } else if b.gain > 0. {
                p.accent
            } else {
                theme::hex(0xe0704a, 0.95)
            };
            let gain_txt = if gainless(b.filter) {
                "—".to_string()
            } else {
                format!("{:+.1}dB", b.gain)
            };
            strip = strip.child(
                div()
                    .id(ElementId::Name(format!("peq-col-{i}").into()))
                    .flex_1()
                    .min_w_0()
                    .py(px(7.))
                    .rounded(px(10.))
                    .cursor_pointer()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(2.))
                    .bg(if on {
                        theme::alpha(col, 0.16)
                    } else {
                        theme::alpha(p.faint, 0.06)
                    })
                    .border_1()
                    .border_color(if on { col } else { p.hair })
                    .hover(|s| s.bg(theme::alpha(p.faint, 0.12)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(div().size(px(7.)).rounded_full().bg(col))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if on { p.fg } else { p.sub })
                                    .child(format!("{} {}", i + 1, b.filter.label())),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.fg)
                            .child(format!("{}Hz", fmt_freq(b.freq))),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(gain_col)
                            .child(gain_txt),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(p.faint)
                            .child(format!("Q {:.2}", b.q)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.peq_sel = i;
                        cx.notify();
                    })),
            );
        }
        strip
    }

    // ------------------------------------------------------------ 選択バンドの編集

    fn peq_editor(
        &mut self,
        pc: &PeqCaps,
        bands: &[PeqBand],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = theme::pal();
        let sel = self.peq_sel;
        let n = bands.len().max(1);
        let Some(band) = bands.get(sel).copied() else {
            return div().child(ui::faint("バンドを読み取れません"));
        };
        let send = Rc::new(
            move |this: &mut Shell, nb: PeqBand, cx: &mut Context<Shell>| {
                this.apply(
                    Change::Peq {
                        band: sel,
                        value: nb,
                    },
                    cx,
                );
            },
        );
        let col = band_color(sel);
        // 1 行目: バンド切替 + タイプ + リセット
        let fl: Vec<String> = pc.filters.iter().map(|f| f.label().to_string()).collect();
        let cur = pc
            .filters
            .iter()
            .position(|f| *f == band.filter)
            .unwrap_or(0);
        let filters = pc.filters.clone();
        let s_type = send.clone();
        let s_zero = send.clone();
        let head = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(ui::icon_button(
                "peq-prev",
                "chevron-left",
                cx,
                move |this, _, cx| {
                    this.peq_sel = (sel + n - 1) % n;
                    cx.notify();
                },
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_w(px(170.))
                    .child(div().size(px(10.)).rounded_full().bg(col))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("バンド {}", sel + 1)),
                    )
                    .child(ui::faint(filter_name(band.filter))),
            )
            .child(ui::icon_button(
                "peq-next",
                "chevron-right",
                cx,
                move |this, _, cx| {
                    this.peq_sel = (sel + 1) % n;
                    cx.notify();
                },
            ))
            .child(div().flex_1())
            .child(div().w(px(260.)).child(ui::segmented(
                "peq-type",
                &fl,
                cur,
                cx,
                move |this, i, cx| {
                    let mut nb = band;
                    nb.filter = filters[i];
                    s_type(this, nb, cx);
                },
            )))
            .child(ui::button(
                "peq-zero",
                "0dB",
                Some("rotate-ccw"),
                Btn::Ghost,
                cx,
                move |this, _, cx| {
                    let mut nb = band;
                    nb.gain = 0.;
                    s_zero(this, nb, cx);
                },
            ));

        // 2 行目: 周波数 / Q / ゲイン を 3 列で
        let field =
            |label: &str, hint: Option<&str>, input: Entity<InputState>, slider: Stateful<Div>| {
                let mut lab = div().flex().flex_col().child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.sub)
                        .child(label.to_string()),
                );
                if let Some(h) = hint {
                    lab = lab.child(
                        div()
                            .text_size(px(10.))
                            .text_color(p.faint)
                            .child(h.to_string()),
                    );
                }
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(px(10.))
                    .bg(theme::alpha(p.faint, 0.05))
                    .border_1()
                    .border_color(p.hair)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .child(lab)
                            .child(
                                div()
                                    .w(px(84.))
                                    .flex_none()
                                    .child(gpui_kit::component::input::Input::new(&input)),
                            ),
                    )
                    .child(slider)
            };
        let (fmin, fmax) = (pc.freq_min as f32, pc.freq_max as f32);
        let (fmin_u, fmax_u) = (pc.freq_min, pc.freq_max);
        let fid = format!("peq{sel}-freq");
        let s1 = send.clone();
        let inp_f = self.num_field(
            &fid,
            band.freq as f32,
            fmin,
            fmax,
            1.,
            "Hz",
            window,
            cx,
            Rc::new(move |this, nv, cx| {
                let mut nb = band;
                nb.freq = nv.round() as u16;
                s1(this, nb, cx);
            }),
        );
        let s2 = send.clone();
        let sl_f = ui::slider_curve(
            &fid,
            band.freq as f32,
            fmin,
            fmax,
            1.,
            2.7,
            Some(col),
            false,
            cx,
            move |this, nv, cx| {
                let mut nb = band;
                nb.freq = (round_freq(nv as f64) as u16).clamp(fmin_u, fmax_u);
                s2(this, nb, cx);
            },
        );
        let qid = format!("peq{sel}-q");
        let s3 = send.clone();
        let inp_q = self.num_field(
            &qid,
            band.q,
            pc.q_min,
            pc.q_max,
            0.01,
            "",
            window,
            cx,
            Rc::new(move |this, nv, cx| {
                let mut nb = band;
                nb.q = nv;
                s3(this, nb, cx);
            }),
        );
        let s4 = send.clone();
        let sl_q = ui::slider_curve(
            &qid,
            band.q,
            pc.q_min,
            pc.q_max,
            0.01,
            2.0,
            Some(col),
            false,
            cx,
            move |this, nv, cx| {
                let mut nb = band;
                nb.q = nv;
                s4(this, nb, cx);
            },
        );
        let gid = format!("peq{sel}-gain");
        let s5 = send.clone();
        let inp_g = self.num_field(
            &gid,
            band.gain,
            pc.gain_min,
            pc.gain_max,
            0.1,
            "dB",
            window,
            cx,
            Rc::new(move |this, nv, cx| {
                let mut nb = band;
                nb.gain = nv;
                s5(this, nb, cx);
            }),
        );
        let s6 = send.clone();
        let sl_g = ui::slider(
            &gid,
            band.gain,
            pc.gain_min,
            pc.gain_max,
            0.1,
            Some(col),
            cx,
            move |this, nv, cx| {
                let mut nb = band;
                nb.gain = nv;
                s6(this, nb, cx);
            },
        );
        let q_hint = if band.filter == PeqFilter::LowShelf {
            "この機種のローシェルフは Q を使いません"
        } else {
            "大きいほど狭い帯域"
        };
        let g_hint = if gainless(band.filter) {
            Some("ローパス/ハイパスはゲインを使いません")
        } else {
            None
        };
        let f_q = field("Q", Some(q_hint), inp_q, sl_q);
        let f_g = field("ゲイン", g_hint, inp_g, sl_g);
        let row = div()
            .flex()
            .gap(px(10.))
            .child(field("周波数", Some("20Hz〜20kHz"), inp_f, sl_f))
            .child(if band.filter == PeqFilter::LowShelf {
                f_q.opacity(0.55)
            } else {
                f_q
            })
            .child(if gainless(band.filter) {
                f_g.opacity(0.55)
            } else {
                f_g
            });
        div().flex().flex_col().gap(px(10.)).child(head).child(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn apo_roundtrip() {
        let txt = "Preamp: -6.4 dB\nFilter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.70\nFilter 2: ON PK Fc 3000 Hz Gain -2.1 dB Q 1.40\nFilter 3: OFF PK Fc 1 Hz Gain 0 dB Q 1\nFilter 4: ON HSC Fc 10000 Hz Gain -3.0 dB Q 0.70\n";
        let (pre, list, skipped) = parse_apo(txt);
        assert_eq!(pre, Some(-6.4));
        assert_eq!(list.len(), 3);
        assert_eq!(skipped, 1);
        assert_eq!(list[0].0, PeqFilter::LowShelf);
        assert_eq!(list[1], (PeqFilter::Peak, 3000, 1.4, -2.1));
        let pc = PeqCaps {
            bands: 10,
            freq_min: 20,
            freq_max: 20000,
            q_min: 0.5,
            q_max: 10.,
            gain_min: -12.,
            gain_max: 12.,
            filters: vec![PeqFilter::LowShelf, PeqFilter::Peak, PeqFilter::HighShelf],
        };
        let b = fill_bands(10, &list, &pc);
        assert_eq!(b.len(), 10);
        let out = to_apo(&b[..3], Some(-6.4));
        let (_, l2, _) = parse_apo(&out);
        assert_eq!(l2.len(), 3);
        assert_eq!(fmt_freq(1000), "1k");
        assert_eq!(fmt_freq(2500), "2.5k");
        assert_eq!(fmt_freq(16000), "16k");
    }
}

//! マウス: DPI とボタン割り当て。

use crate::app::{Modal, Shell};
use crate::ui::{self, Btn};
use crate::{keycode, motion, overlay, theme};
use devhub_core::keys;
use devhub_core::*;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::*;

const PRESETS: &[u32] = &[400, 800, 1200, 1600, 3200, 6400];
const DEFAULT_COLORS: &[u32] = &[
    0xff3040, 0x2f6bff, 0x22d36b, 0xff3df5, 0xffd21a, 0xffffff, 0x2ee6ff, 0xff8a3d,
];

fn mouse_parts(this: &Shell) -> Option<(MouseCaps, MouseState)> {
    let s = this.cur()?;
    Some((
        s.caps.as_ref()?.mouse.clone()?,
        s.state.as_ref()?.mouse.clone()?,
    ))
}

/// DPI 入力欄の確定。
pub fn commit_dpi_text(this: &mut Shell, v: &str, y_axis: bool, cx: &mut Context<Shell>) {
    let Some((mc, ms)) = mouse_parts(this) else {
        return;
    };
    let Ok(n) = v.trim().parse::<u32>() else {
        this.toast("数値を入力してください", true, cx);
        return;
    };
    let n = ui::quantize(
        n as f32,
        mc.dpi_min as f32,
        mc.dpi_max as f32,
        mc.dpi_step as f32,
    ) as u32;
    let i = this.dpi_sel.min(ms.stages.len().saturating_sub(1));
    let mut stages = ms.stages.clone();
    if y_axis {
        stages[i].y = Some(n);
    } else {
        stages[i].x = n;
    }
    this.apply(
        Change::DpiStages {
            stages,
            active: ms.active_stage,
        },
        cx,
    );
}

pub fn action_label(a: &ButtonAction, macros: &[Option<MacroDef>]) -> String {
    match a {
        ButtonAction::Disabled => "無効".into(),
        ButtonAction::Mouse(m) => keys::mouse_fn_name(*m).into(),
        ButtonAction::Dpi(DpiFn::Cycle) => "DPI 循環".into(),
        ButtonAction::Dpi(DpiFn::Up) => "DPI +".into(),
        ButtonAction::Dpi(DpiFn::Down) => "DPI −".into(),
        ButtonAction::Dpi(DpiFn::Shift(v)) => format!("DPI ロック ({v})"),
        ButtonAction::Fire {
            button,
            count,
            interval_ms,
        } => {
            format!(
                "連射: {} ×{count} / {interval_ms}ms",
                keys::mouse_fn_name(*button)
            )
        }
        ButtonAction::Key(k) => format!("キー: {}", keycode::stroke_label(*k)),
        ButtonAction::Media(m) => keys::media_fn_name(*m).into(),
        ButtonAction::Macro { slot, run } => {
            let n = macros
                .get(*slot as usize)
                .and_then(|m| m.as_ref())
                .map(|m| m.name.clone())
                .unwrap_or_else(|| format!("マクロ {}", slot + 1));
            let r = match run {
                MacroRun::Times(t) => format!("{t} 回"),
                MacroRun::WhileHeld => "押している間".into(),
                MacroRun::Toggle => "トグル".into(),
            };
            format!("マクロ: {n} ({r})")
        }
        ButtonAction::PollingCycle => "ポーリング切替".into(),
        ButtonAction::ProfileCycle => "プロファイル切替".into(),
        ButtonAction::ScrollModeToggle => "上下ロール切替".into(),
        ButtonAction::Unsupported { ty, .. } => format!("未対応の割当 (type {ty})"),
    }
}

impl Shell {
    // ================================================================== DPI

    pub fn page_dpi(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let Some((mc, ms)) = mouse_parts(self) else {
            return div().into_any_element();
        };
        if ms.stages.is_empty() {
            return ui::card()
                .child(ui::empty_state("crosshair", "DPI ステージがありません", ""))
                .into_any_element();
        }
        // 未選択なら現在使用中のステージを初期表示にする (選択したら追従を止める)。
        if !self.dpi_sel_touch {
            self.dpi_sel = ms.active_stage;
        }
        self.dpi_sel = self.dpi_sel.min(ms.stages.len() - 1);
        let sel = self.dpi_sel;
        let st = ms.stages[sel];
        let xy = ms.stages.iter().any(|s| s.y.is_some());
        let (min, max, step) = (mc.dpi_min as f32, mc.dpi_max as f32, mc.dpi_step as f32);

        // 入力欄は選択が変わったら値を入れ替える。
        let key_x = format!("{}:{}", sel, st.x);
        if INPUT_SYNC.with(|c| c.borrow().as_deref() != Some(key_x.as_str())) {
            INPUT_SYNC.with(|c| *c.borrow_mut() = Some(key_x));
            self.input("dpi_exact", "", "DPI", window, cx);
            self.set_input("dpi_exact", &st.x.to_string(), window, cx);
            self.input("dpi_exact_y", "", "Y", window, cx);
            self.set_input("dpi_exact_y", &st.y.unwrap_or(st.x).to_string(), window, cx);
        }
        let in_x = self.input("dpi_exact", "", "DPI", window, cx);
        let in_y = self.input("dpi_exact_y", "", "Y", window, cx);

        // ---- 上段: 大きな数字
        let shown = motion::spring_with(&format!("dpi-num:{sel}"), st.x as f32, 18.);
        let color = theme::rgb_of(st.color);
        let hero = div()
            .relative()
            .flex()
            .items_end()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(ui::faint(format!("ステージ {} を編集中", sel + 1)))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(10.))
                            .child(ui::big_number(format!("{}", shown.round() as u32), 64.))
                            .when(xy, |d| {
                                d.child(
                                    div()
                                        .pb(px(10.))
                                        .text_size(px(20.))
                                        .text_color(p.sub)
                                        .child(format!("× {}", st.y.unwrap_or(st.x))),
                                )
                            })
                            .child(
                                div()
                                    .pb(px(12.))
                                    .text_size(px(14.))
                                    .text_color(p.faint)
                                    .child("DPI"),
                            ),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(ms.active_stage == sel, |d| {
                        d.child(ui::chip("使用中", p.ok))
                    })
                    .when(ms.active_stage != sel, |d| {
                        d.child(ui::button(
                            "use-stage",
                            "このステージを使う",
                            Some("check"),
                            Btn::Glass,
                            cx,
                            move |this, _, cx| {
                                this.apply(Change::ActiveStage(sel), cx);
                            },
                        ))
                    }),
            );

        // ---- ステージ一覧 (縦棒 + 値)
        let maxv = ms
            .stages
            .iter()
            .map(|s| s.x.max(s.y.unwrap_or(0)))
            .max()
            .unwrap_or(1)
            .max(1600) as f32;
        let mut bars = div().flex().items_end().gap(px(10.)).h(px(150.));
        for (i, s) in ms.stages.iter().enumerate() {
            let on = i == sel;
            let active = i == ms.active_stage;
            let hgt = motion::spring_with(
                &format!("dpibar:{i}"),
                (s.x as f32 / maxv).sqrt() * 96. + 8.,
                18.,
            );
            let c = theme::rgb_of(s.color);
            bars = bars.child(
                div()
                    .id(SharedString::from(format!("stage-{i}")))
                    .w(px(64.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(if on {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if on { p.fg } else { p.sub })
                            .child(format!("{}", s.x)),
                    )
                    .child(
                        div()
                            .w(px(if on { 34. } else { 26. }))
                            .h(px(hgt))
                            .rounded(px(10.))
                            .bg(linear_gradient(
                                180.,
                                linear_color_stop(c, 0.),
                                linear_color_stop(theme::alpha(c, 0.25), 1.),
                            ))
                            .border_1()
                            .border_color(if on { p.fg } else { theme::alpha(c, 0.6) })
                            .shadow(vec![BoxShadow {
                                color: theme::alpha(c, if on { 0.55 } else { 0.25 }),
                                offset: point(px(0.), px(6.)),
                                blur_radius: px(18.),
                                spread_radius: px(-6.),
                                inset: false,
                            }]),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .text_size(px(11.))
                            .text_color(if active { p.ok } else { p.faint })
                            .when(active, |d| {
                                d.child(div().size(px(5.)).rounded(px(3.)).bg(p.ok))
                            })
                            .child(format!("{}", i + 1)),
                    )
                    .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                        this.dpi_sel = i;
                        this.dpi_sel_touch = true;
                        // ダブルクリックでそのステージを使用。
                        if let ClickEvent::Mouse(m) = e {
                            if m.down.click_count >= 2 {
                                this.apply(Change::ActiveStage(i), cx);
                            }
                        }
                        cx.notify();
                    })),
            );
        }
        let can_add = ms.stages.len() < mc.max_stages;
        let can_del = ms.stages.len() > mc.min_stages.max(1);
        let stages_add = ms.stages.clone();
        let active_add = ms.active_stage;
        let stages_del = ms.stages.clone();
        let active_del = ms.active_stage;
        let stage_tools = div()
            .flex()
            .gap(px(6.))
            .when(can_add, |d| {
                d.child(ui::button(
                    "stage-add",
                    "追加",
                    Some("plus"),
                    Btn::Glass,
                    cx,
                    move |this, _, cx| {
                        let mut s = stages_add.clone();
                        let last = *s.last().expect("stage");
                        let n = s.len();
                        s.push(DpiStage {
                            x: (last.x * 2).min(mc.dpi_max),
                            y: last.y.map(|y| (y * 2).min(mc.dpi_max)),
                            color: Rgb::hex(DEFAULT_COLORS[n % DEFAULT_COLORS.len()]),
                            enabled: true,
                        });
                        this.dpi_sel = n;
                        this.dpi_sel_touch = true;
                        this.apply(
                            Change::DpiStages {
                                stages: s,
                                active: active_add,
                            },
                            cx,
                        );
                    },
                ))
            })
            .when(can_del, |d| {
                d.child(ui::button(
                    "stage-del",
                    "削除",
                    Some("trash"),
                    Btn::Ghost,
                    cx,
                    move |this, _, cx| {
                        let mut s = stages_del.clone();
                        let i = this.dpi_sel.min(s.len() - 1);
                        s.remove(i);
                        let active = if active_del >= s.len() {
                            s.len() - 1
                        } else if active_del > i {
                            active_del - 1
                        } else {
                            active_del
                        };
                        this.dpi_sel = i.min(s.len() - 1);
                        this.dpi_sel_touch = true;
                        this.apply(Change::DpiStages { stages: s, active }, cx);
                    },
                ))
            });

        // ---- 編集
        let stages_x = ms.stages.clone();
        let active = ms.active_stage;
        let slider_x = ui::slider_curve(
            "dpi-x",
            st.x as f32,
            min,
            max,
            step,
            2.4,
            Some(color),
            cx,
            move |this, v, cx| {
                let mut s = stages_x.clone();
                let i = this.dpi_sel.min(s.len() - 1);
                if s[i].x == v as u32 {
                    return;
                }
                s[i].x = v as u32;
                this.apply(Change::DpiStages { stages: s, active }, cx);
            },
        );
        let mut presets = div().flex().gap(px(6.));
        for pv in PRESETS
            .iter()
            .copied()
            .filter(|v| *v >= mc.dpi_min && *v <= mc.dpi_max)
        {
            let stages_p = ms.stages.clone();
            let on = st.x == pv;
            presets = presets.child(
                div()
                    .id(SharedString::from(format!("dpi-preset-{pv}")))
                    .h(px(28.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(9.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .border_1()
                    .when(on, |d| {
                        d.bg(p.accent_soft)
                            .border_color(theme::alpha(p.accent, 0.5))
                    })
                    .when(!on, |d| {
                        d.bg(p.ctrl)
                            .border_color(p.hair)
                            .hover(|s| s.bg(p.ctrl_hover))
                    })
                    .child(format!("{pv}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let mut s = stages_p.clone();
                        let i = this.dpi_sel.min(s.len() - 1);
                        s[i].x = pv;
                        if s[i].y.is_some() {
                            s[i].y = Some(pv);
                        }
                        this.apply(Change::DpiStages { stages: s, active }, cx);
                    })),
            );
        }

        let mut edit = ui::body().pt(px(4.)).child(ui::row_first(
            if xy { "X 軸 DPI" } else { "DPI" },
            None,
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .w(px(480.))
                .child(slider_x)
                .child(div().w(px(96.)).child(Input::new(&in_x))),
        ));
        if xy {
            let stages_y = ms.stages.clone();
            edit = edit.child(ui::row(
                "Y 軸 DPI",
                None,
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .w(px(480.))
                    .child(ui::slider_curve(
                        "dpi-y",
                        st.y.unwrap_or(st.x) as f32,
                        min,
                        max,
                        step,
                        2.4,
                        Some(color),
                        cx,
                        move |this, v, cx| {
                            let mut s = stages_y.clone();
                            let i = this.dpi_sel.min(s.len() - 1);
                            s[i].y = Some(v as u32);
                            this.apply(Change::DpiStages { stages: s, active }, cx);
                        },
                    ))
                    .child(div().w(px(96.)).child(Input::new(&in_y))),
            ));
        }
        edit = edit.child(ui::row("プリセット", None, presets));
        if mc.xy_split {
            let stages_t = ms.stages.clone();
            edit = edit.child(ui::row(
                "X / Y を個別に設定",
                Some("縦と横で感度を変えます"),
                ui::toggle("xy-split", xy, cx, move |this, on, cx| {
                    let s: Vec<DpiStage> = stages_t
                        .iter()
                        .map(|st| DpiStage {
                            y: if on { Some(st.x) } else { None },
                            ..*st
                        })
                        .collect();
                    this.apply(Change::DpiStages { stages: s, active }, cx);
                }),
            ));
        }
        if mc.stage_colors {
            let stages_c = ms.stages.clone();
            let cur = st.color;
            edit = edit.child(ui::row(
                "インジケーターの色",
                Some("このステージのとき LED が光る色"),
                div()
                    .id("stage-color")
                    .cursor_pointer()
                    .child(ui::swatch(color, 26., false))
                    .on_click(cx.listener(move |this, _, w, cx| {
                        let stages_c = stages_c.clone();
                        overlay::pick_color(
                            this,
                            "インジケーターの色",
                            cur,
                            w,
                            cx,
                            move |this, c, _, cx| {
                                let mut s = stages_c.clone();
                                let i = this.dpi_sel.min(s.len() - 1);
                                s[i].color = c;
                                this.apply(Change::DpiStages { stages: s, active }, cx);
                            },
                        );
                    })),
            ));
        }

        let stage_card = ui::card_titled(
            "DPI ステージ",
            "crosshair",
            Some(stage_tools.into_any_element()),
        )
        .child(
            ui::body()
                .gap(px(14.))
                .pt(px(6.))
                .child(hero)
                .child(bars)
                .child(ui::faint(
                    "クリックで編集 · ダブルクリックでそのステージに切り替え",
                )),
        );
        let edit_card = ui::card_titled(
            &format!("ステージ {} の設定", sel + 1),
            "sliders-horizontal",
            None,
        )
        .child(edit);
        div()
            .flex()
            .flex_col()
            .gap(px(theme::GAP))
            .child(stage_card)
            .child(edit_card)
            .into_any_element()
    }

    // ================================================================== ボタン

    pub fn page_buttons(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let Some((mc, ms)) = mouse_parts(self) else {
            return div().into_any_element();
        };
        let hover_slot = (0..mc.buttons.len()).find(|i| motion::hovered(&format!("btnrow:{i}")));

        // マウスの図。
        let mark = |n: usize, x: f32, y: f32, hot: bool| {
            div()
                .absolute()
                .left(px(x - 12.))
                .top(px(y - 12.))
                .size(px(24.))
                .rounded(px(12.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .font_weight(FontWeight::BOLD)
                .bg(if hot {
                    p.accent
                } else {
                    theme::alpha(p.fg, 0.14)
                })
                .text_color(if hot { p.on_accent } else { p.fg })
                .border_1()
                .border_color(p.edge)
                .child(format!("{}", n + 1))
        };
        let positions = [
            (70., 70.),
            (150., 70.),
            (110., 62.),
            (34., 150.),
            (34., 112.),
        ];
        let mut figure = div()
            .relative()
            .w(px(220.))
            .h(px(300.))
            .child(
                // 胴体
                div()
                    .absolute()
                    .left(px(40.))
                    .top(px(20.))
                    .w(px(140.))
                    .h(px(250.))
                    .rounded_t(px(70.))
                    .rounded_b(px(62.))
                    .bg(linear_gradient(
                        160.,
                        linear_color_stop(theme::alpha(p.fg, 0.16), 0.),
                        linear_color_stop(theme::alpha(p.fg, 0.05), 1.),
                    ))
                    .border_1()
                    .border_color(p.edge),
            )
            .child(
                div()
                    .absolute()
                    .left(px(110.))
                    .top(px(22.))
                    .w(px(1.))
                    .h(px(90.))
                    .bg(p.hair),
            )
            .child(
                // ホイール
                div()
                    .absolute()
                    .left(px(103.))
                    .top(px(44.))
                    .w(px(14.))
                    .h(px(34.))
                    .rounded(px(7.))
                    .bg(theme::alpha(p.fg, 0.25)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(36.))
                    .top(px(100.))
                    .w(px(6.))
                    .h(px(24.))
                    .rounded(px(3.))
                    .bg(theme::alpha(p.fg, 0.3)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(36.))
                    .top(px(138.))
                    .w(px(6.))
                    .h(px(24.))
                    .rounded(px(3.))
                    .bg(theme::alpha(p.fg, 0.3)),
            );
        for (i, (x, y)) in positions.iter().enumerate().take(mc.buttons.len()) {
            figure = figure.child(mark(i, *x, *y, hover_slot == Some(i)));
        }

        let mut list = ui::body().pt(px(4.));
        for (i, slot) in mc.buttons.iter().enumerate() {
            let a = ms.buttons.get(i).cloned().unwrap_or(ButtonAction::Disabled);
            let label = action_label(&a, &ms.macros);
            let changed = !matches!(
                (i, &a),
                (0, ButtonAction::Mouse(MouseFn::Left))
                    | (1, ButtonAction::Mouse(MouseFn::Right))
                    | (2, ButtonAction::Mouse(MouseFn::Middle))
                    | (3, ButtonAction::Mouse(MouseFn::Back))
                    | (4, ButtonAction::Mouse(MouseFn::Forward))
            );
            let key = format!("btnrow:{i}");
            let a2 = a.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("btn-slot-{i}")))
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(10.))
                    .py(px(10.))
                    .my(px(2.))
                    .rounded(px(14.))
                    .cursor_pointer()
                    .hover(|s| s.bg(p.ctrl))
                    .child(motion::track(key))
                    .child(
                        div()
                            .size(px(26.))
                            .rounded(px(13.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(11.))
                            .font_weight(FontWeight::BOLD)
                            .bg(if hover_slot == Some(i) {
                                p.accent
                            } else {
                                p.ctrl_hover
                            })
                            .text_color(if hover_slot == Some(i) {
                                p.on_accent
                            } else {
                                p.fg
                            })
                            .child(format!("{}", i + 1)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(slot.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(if changed { p.accent } else { p.sub })
                                    .child(label),
                            ),
                    )
                    .child(ui::icon("chevron-right", 15., p.faint))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.modal = Some(Modal::Button {
                            slot: i,
                            draft: a2.clone(),
                            capture: false,
                        });
                        cx.notify();
                    })),
            );
        }
        let defaults: Vec<ButtonAction> = vec![
            ButtonAction::Mouse(MouseFn::Left),
            ButtonAction::Mouse(MouseFn::Right),
            ButtonAction::Mouse(MouseFn::Middle),
            ButtonAction::Mouse(MouseFn::Back),
            ButtonAction::Mouse(MouseFn::Forward),
        ];
        let n = mc.buttons.len();
        let reset = ui::button(
            "btn-reset",
            "既定に戻す",
            Some("rotate-ccw"),
            Btn::Ghost,
            cx,
            move |this, _, cx| {
                let d = defaults.clone();
                this.confirm(
                    "ボタン割り当てを既定に戻す",
                    if n > d.len() {
                        "一覧の先頭5ボタンを既定の割り当てに戻します。"
                    } else {
                        "すべてのボタンを工場出荷時の割り当てに戻します。"
                    },
                    "戻す",
                    false,
                    std::rc::Rc::new(move |this, _, cx| {
                        for (i, a) in d.iter().enumerate().take(n) {
                            this.apply(
                                Change::Button {
                                    slot: i,
                                    action: a.clone(),
                                },
                                cx,
                            );
                        }
                    }),
                    cx,
                );
            },
        );

        div()
            .flex()
            .items_start()
            .gap(px(theme::GAP))
            .child(
                ui::card().w(px(280.)).flex_none().child(
                    div()
                        .relative()
                        .p(px(24.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(8.))
                        .child(figure)
                        .child(ui::faint("上から見た図 · 番号は右の一覧に対応")),
                ),
            )
            .child(
                ui::card_titled(
                    "ボタン割り当て",
                    "mouse-pointer-click",
                    Some(reset.into_any_element()),
                )
                .flex_1()
                .child(list),
            )
            .into_any_element()
    }
}

thread_local! {
    static INPUT_SYNC: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

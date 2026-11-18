//! 部品 (カード・行・トグル・スライダー・セグメント・ドロップダウン・ボタン…)。
//!
//! 部品はすべて `Shell` を相手にしたリスナーを作るので `cx: &mut Context<Shell>` を取る。

use crate::app::{CbF, PopItem, Shell};
use crate::{glass, motion, theme};
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

// ====================================================================== 文字

pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

pub fn title(text: impl Into<SharedString>) -> Div {
    let p = theme::pal();
    div()
        .text_size(px(28.))
        .line_height(px(34.))
        .font_weight(FontWeight::BOLD)
        .text_color(p.fg)
        .child(text.into())
}

pub fn section_label(text: &str) -> Div {
    let p = theme::pal();
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.faint)
        .child(text.to_string())
}

pub fn sub(text: impl Into<SharedString>) -> Div {
    let p = theme::pal();
    div()
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(p.sub)
        .child(text.into())
}

pub fn faint(text: impl Into<SharedString>) -> Div {
    let p = theme::pal();
    div()
        .text_size(px(11.5))
        .line_height(px(16.))
        .text_color(p.faint)
        .child(text.into())
}

/// 大きな数字 (DPI など)。
pub fn big_number(text: impl Into<SharedString>, size: f32) -> Div {
    let p = theme::pal();
    div()
        .font(theme::display_font())
        .text_size(px(size))
        .line_height(px(size * 1.05))
        .font_weight(FontWeight::BOLD)
        .text_color(p.fg)
        .child(text.into())
}

// ====================================================================== 面

/// ガラスのカード。子を足して使う。
pub fn card() -> Div {
    let p = theme::pal();
    div()
        .relative()
        .rounded(px(theme::R_CARD))
        .shadow(vec![BoxShadow {
            color: theme::alpha(p.shadow, p.shadow.a * 0.8),
            offset: point(px(0.), px(10.)),
            blur_radius: px(32.),
            spread_radius: px(-12.),
            inset: false,
        }])
        .child(glass::surface(glass::Kind::Panel, theme::R_CARD))
}

/// 見出し付きカード。`body` に行を足す。
pub fn card_titled(title: &str, icon_name: &str, trailing: Option<AnyElement>) -> Div {
    let p = theme::pal();
    card().child(
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(9.))
            .px(px(18.))
            .pt(px(16.))
            .pb(px(6.))
            .child(
                div()
                    .size(px(26.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(p.accent_soft)
                    .child(icon(icon_name, 14., p.accent)),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.fg)
                    .child(title.to_string()),
            )
            .when_some(trailing, |d, t| d.child(t)),
    )
}

/// カードの中身の器 (左右の余白)。
pub fn body() -> Div {
    div().relative().flex().flex_col().px(px(18.)).pb(px(12.))
}

/// 設定の 1 行。
pub fn row(label: &str, help: Option<&str>, control: impl IntoElement) -> Div {
    let p = theme::pal();
    div()
        .relative()
        .flex()
        .items_center()
        .gap(px(16.))
        .py(px(11.))
        .border_t_1()
        .border_color(p.hair)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(p.fg)
                        .child(label.to_string()),
                )
                .when_some(help, |d, h| d.child(faint(h.to_string()))),
        )
        .child(div().flex_none().flex().items_center().child(control))
}

/// 区切りの無い行 (カードの先頭用)。
pub fn row_first(label: &str, help: Option<&str>, control: impl IntoElement) -> Div {
    row(label, help, control).border_t_0()
}

pub fn chip(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .px(px(8.))
        .h(px(20.))
        .flex()
        .items_center()
        .rounded(px(10.))
        .bg(theme::alpha(color, 0.16))
        .border_1()
        .border_color(theme::alpha(color, 0.3))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(text.into())
}

pub fn sync_dot(pending: bool) -> Div {
    let p = theme::pal();
    let c = if pending { p.warn } else { p.ok };
    div()
        .size(px(7.))
        .rounded(px(4.))
        .bg(c)
        .shadow(vec![BoxShadow {
            color: theme::alpha(c, 0.7),
            offset: point(px(0.), px(0.)),
            blur_radius: px(6.),
            spread_radius: px(0.),
            inset: false,
        }])
}

pub fn battery_pill(pct: u8, charging: bool) -> Div {
    let p = theme::pal();
    // 4 段階の残量アイコン。充電中は稲妻、20% 以下は橙、10% 以下は赤。
    let (ic, c) = if charging {
        ("battery-charging", p.ok)
    } else if pct <= 10 {
        ("battery-warning", p.danger)
    } else if pct <= 20 {
        ("battery", p.warn)
    } else if pct > 75 {
        ("battery-full", p.fg)
    } else if pct > 50 {
        ("battery-medium", p.fg)
    } else {
        ("battery-low", p.fg)
    };
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(11.))
        .text_color(if charging { p.ok } else { p.sub })
        .child(icon(ic, 14., c))
        .child(if charging {
            "充電中".to_string()
        } else {
            format!("{pct}%")
        })
}

pub fn swatch(c: Hsla, size: f32, selected: bool) -> Div {
    let p = theme::pal();
    div()
        .size(px(size))
        .rounded(px(size / 2.))
        .bg(c)
        .border_2()
        .border_color(if selected {
            p.fg
        } else {
            theme::alpha(p.fg, 0.12)
        })
        .shadow(vec![BoxShadow {
            color: theme::alpha(c, 0.45),
            offset: point(px(0.), px(2.)),
            blur_radius: px(8.),
            spread_radius: px(-2.),
            inset: false,
        }])
}

pub fn empty_state(icon_name: &str, title_text: &str, body_text: &str) -> Div {
    let p = theme::pal();
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.))
        .py(px(40.))
        .child(
            div()
                .size(px(56.))
                .rounded(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .bg(p.ctrl)
                .border_1()
                .border_color(p.hair)
                .child(icon(icon_name, 26., p.sub)),
        )
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title_text.to_string()),
        )
        .child(
            div()
                .max_w(px(420.))
                .text_center()
                .text_size(px(12.5))
                .text_color(p.sub)
                .child(body_text.to_string()),
        )
}

// ====================================================================== ボタン

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Primary,
    Glass,
    Ghost,
    Danger,
}

pub fn button(
    id: impl Into<SharedString>,
    label: &str,
    icon_name: Option<&str>,
    style: Btn,
    cx: &mut Context<Shell>,
    on: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) -> Stateful<Div> {
    let p = theme::pal();
    let id: SharedString = id.into();
    let (fg, bg): (Hsla, Option<Background>) = match style {
        Btn::Primary => (p.on_accent, Some(theme::accent_grad())),
        Btn::Danger => (p.danger, Some(theme::alpha(p.danger, 0.12).into())),
        Btn::Glass => (p.fg, None),
        Btn::Ghost => (p.sub, None),
    };
    let mut d = div()
        .id(id.clone())
        .relative()
        .h(px(34.))
        .px(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(7.))
        .rounded(px(theme::R_CTRL))
        .cursor_pointer()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg);
    match style {
        Btn::Glass => {
            d = d.child(glass::surface_hover(
                glass::Kind::Frost,
                theme::R_CTRL,
                id.to_string(),
            ));
        }
        Btn::Ghost => {
            d = d.hover(|s| s.bg(p.ctrl));
        }
        Btn::Primary => {
            d = d
                .when_some(bg, |d, b| d.bg(b))
                .shadow(vec![BoxShadow {
                    color: theme::alpha(p.accent, 0.4),
                    offset: point(px(0.), px(6.)),
                    blur_radius: px(16.),
                    spread_radius: px(-6.),
                    inset: false,
                }])
                .hover(|s| s.opacity(0.92));
        }
        Btn::Danger => {
            d = d
                .when_some(bg, |d, b| d.bg(b))
                .border_1()
                .border_color(theme::alpha(p.danger, 0.3))
                .hover(|s| s.bg(theme::alpha(p.danger, 0.2)));
        }
    }
    d.when_some(icon_name, |d, n| {
        d.child(div().relative().child(icon(n, 15., fg)))
    })
    .when(!label.is_empty(), |d| {
        d.child(div().relative().child(label.to_string()))
    })
    .on_click(cx.listener(move |this, _, w, cx| on(this, w, cx)))
}

/// 丸いアイコンボタン。
pub fn icon_button(
    id: impl Into<SharedString>,
    icon_name: &str,
    cx: &mut Context<Shell>,
    on: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) -> Stateful<Div> {
    let p = theme::pal();
    div()
        .id(id.into())
        .size(px(30.))
        .rounded(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(p.ctrl_hover))
        .child(icon(icon_name, 15., p.sub))
        .on_click(cx.listener(move |this, _, w, cx| on(this, w, cx)))
}

// ====================================================================== トグル

pub fn toggle(
    id: impl Into<SharedString>,
    on: bool,
    cx: &mut Context<Shell>,
    f: impl Fn(&mut Shell, bool, &mut Context<Shell>) + 'static,
) -> Stateful<Div> {
    let p = theme::pal();
    let id: SharedString = id.into();
    let k = motion::ease(&format!("tg:{id}"), on);
    let pressed_key = format!("tgp:{id}");
    let pressed = motion::is_down() && motion::hovered(&pressed_key);
    let squash = motion::ease(&format!("tgs:{id}"), pressed);
    let (w, h) = (46., 28.);
    let knob = 22. + squash * 6.;
    let x = 3. + k * (w - 6. - knob);
    let track = theme::mix(p.track, p.accent, k);
    div()
        .id(id.clone())
        .relative()
        .w(px(w))
        .h(px(h))
        .flex_none()
        .rounded(px(h / 2.))
        .bg(track)
        .cursor_pointer()
        .child(motion::track(pressed_key))
        .child(
            div()
                .absolute()
                .top(px(3.))
                .left(px(x))
                .w(px(knob))
                .h(px(22.))
                .rounded(px(11.))
                .bg(theme::hex(0xffffff, 1. - squash * 0.75))
                .shadow(vec![BoxShadow {
                    color: theme::hex(0x000000, 0.25),
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(6.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .when(squash > 0.05, |d| {
                    d.child(
                        canvas(
                            |b, _, _| b,
                            |_, b, w, _| {
                                let r = f32::from(b.size.height) / 2.;
                                glass::bubble(b, r, w)
                            },
                        )
                        .absolute()
                        .inset_0(),
                    )
                }),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            f(this, !on, cx);
            cx.notify();
        }))
}

// ====================================================================== スライダー

/// 値を step に丸める。
pub fn quantize(v: f32, min: f32, max: f32, step: f32) -> f32 {
    let v = if step > 0. {
        min + ((v - min) / step).round() * step
    } else {
        v
    };
    v.clamp(min, max)
}

pub fn slider(
    id: &str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    color: Option<Hsla>,
    cx: &mut Context<Shell>,
    f: impl Fn(&mut Shell, f32, &mut Context<Shell>) + 'static,
) -> Stateful<Div> {
    slider_curve(id, value, min, max, step, 1., color, false, cx, f)
}

/// `gamma` > 1 で低い値ほど細かく動かせる (DPI など桁が広い値)。
pub fn slider_curve(
    id: &str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    gamma: f32,
    color: Option<Hsla>,
    commit_on_release: bool,
    cx: &mut Context<Shell>,
    f: impl Fn(&mut Shell, f32, &mut Context<Shell>) + 'static,
) -> Stateful<Div> {
    let p = theme::pal();
    let rect_key = format!("sl:{id}");
    let dragging = motion::is_down() && motion::hovered(&format!("slh:{id}"));
    let t = if max > min {
        ((value - min) / (max - min)).clamp(0., 1.).powf(1. / gamma)
    } else {
        0.
    };
    let preview = if commit_on_release && dragging {
        motion::rect(&rect_key)
            .map(|rect| ((motion::pointer()[0] - rect[0]) / rect[2].max(1.)).clamp(0., 1.))
            .unwrap_or(t)
    } else {
        t
    };
    let ts = motion::spring_with(&format!("slv:{id}"), preview, 30.);
    let grow = motion::ease(&format!("slg:{id}"), dragging);
    let accent = color.unwrap_or(p.accent);
    let f = Rc::new(f);
    let apply: CbF = Rc::new(move |this: &mut Shell, t: f32, cx: &mut Context<Shell>| {
        let v = quantize(min + t.powf(gamma) * (max - min), min, max, step);
        f(this, v, cx);
    });
    let thumb_w = 22. + grow * 10.;
    let rk = rect_key.clone();
    div()
        .id(SharedString::from(format!("slider-{id}")))
        .relative()
        .h(px(30.))
        .flex_1()
        .min_w(px(120.))
        .cursor_pointer()
        .child(motion::track(format!("slh:{id}")))
        // トラック
        .child(
            div()
                .absolute()
                .left(px(11.))
                .right(px(11.))
                .top(px(12.))
                .h(px(6.))
                .rounded(px(3.))
                .bg(p.track)
                .child(motion::track(rect_key))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .h_full()
                        .w(relative(ts))
                        .rounded(px(3.))
                        .bg(linear_gradient(
                            90.,
                            linear_color_stop(theme::alpha(accent, 0.55), 0.),
                            linear_color_stop(accent, 1.),
                        )),
                )
                // つまみ
                .child(
                    div()
                        .absolute()
                        .left(relative(ts))
                        .top(px(-8. - grow * 2.))
                        .ml(px(-thumb_w / 2.))
                        .w(px(thumb_w))
                        .h(px(22. + grow * 4.))
                        .rounded(px(13.))
                        .bg(theme::hex(0xffffff, 1. - grow * 0.8))
                        .shadow(vec![BoxShadow {
                            color: theme::hex(0x000000, 0.28),
                            offset: point(px(0.), px(2.)),
                            blur_radius: px(7.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        .when(grow > 0.05, |d| {
                            d.child(
                                canvas(
                                    |b, _, _| b,
                                    |_, b, w, _| {
                                        let r = f32::from(b.size.height) / 2.;
                                        glass::bubble(b, r, w)
                                    },
                                )
                                .absolute()
                                .inset_0(),
                            )
                        }),
                ),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                motion::set_down(true);
                motion::set_pointer(f32::from(e.position.x), f32::from(e.position.y));
                this.begin_drag(&rk, false, commit_on_release, apply.clone(), e.position, cx);
            }),
        )
}

/// スライダー + 値表示の 1 行分の部品。
pub fn slider_value(
    id: &str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    unit: &str,
    cx: &mut Context<Shell>,
    f: impl Fn(&mut Shell, f32, &mut Context<Shell>) + 'static,
) -> Div {
    let p = theme::pal();
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .w(px(270.))
        .child(slider(id, value, min, max, step, None, cx, f))
        .child(
            div()
                .w(px(62.))
                .flex()
                .justify_end()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.fg)
                .child(fmt_num(value, step, unit)),
        )
}

/// `slider_value` + 右側に直接入力できる数値欄。`input` は `Shell::num_field` で用意する
/// (入力欄は単位つき表示文字列を持つので `unit` は `num_field` 側で渡す)。
pub fn slider_value_edit(
    id: &str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    input: &Entity<InputState>,
    cx: &mut Context<Shell>,
    f: impl Fn(&mut Shell, f32, &mut Context<Shell>) + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .w(px(330.))
        .child(slider(id, value, min, max, step, None, cx, f))
        .child(
            div()
                .w(px(94.))
                .child(gpui_kit::component::input::Input::new(input)),
        )
}

pub fn fmt_num(v: f32, step: f32, unit: &str) -> String {
    let dec = if step >= 1. {
        0
    } else if step >= 0.1 {
        1
    } else if step >= 0.01 {
        2
    } else {
        3
    };
    format!("{:.*}{}", dec, v, unit)
}

// ====================================================================== セグメント

pub fn segmented(
    id: &str,
    options: &[String],
    selected: usize,
    cx: &mut Context<Shell>,
    f: impl Fn(&mut Shell, usize, &mut Context<Shell>) + 'static,
) -> Div {
    let p = theme::pal();
    let n = options.len().max(1);
    let x = motion::spring_with(&format!("seg:{id}"), selected as f32, 22.);
    let f = Rc::new(f);
    let mut row = div()
        .relative()
        .flex()
        .flex_row()
        .p(px(3.))
        .rounded(px(12.))
        .bg(p.ctrl)
        .border_1()
        .border_color(p.hair);
    // 動く選択面。
    row = row.child(
        div()
            .absolute()
            .top(px(3.))
            .bottom(px(3.))
            .left(px(3.))
            .right(px(3.))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(x / n as f32))
                    .w(relative(1. / n as f32))
                    .rounded(px(9.))
                    .bg(if p.light {
                        theme::hex(0xffffff, 0.95)
                    } else {
                        theme::hex(0xffffff, 0.16)
                    })
                    .border_1()
                    .border_color(p.edge)
                    .shadow(vec![BoxShadow {
                        color: theme::hex(0x000000, if p.light { 0.12 } else { 0.3 }),
                        offset: point(px(0.), px(2.)),
                        blur_radius: px(8.),
                        spread_radius: px(-2.),
                        inset: false,
                    }]),
            ),
    );
    for (i, o) in options.iter().enumerate() {
        let f = f.clone();
        let on = i == selected;
        row = row.child(
            div()
                .id(SharedString::from(format!("seg-{id}-{i}")))
                .relative()
                .flex_1()
                .h(px(28.))
                .px(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_size(px(12.5))
                .font_weight(if on {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if on { p.fg } else { p.sub })
                .whitespace_nowrap()
                .child(o.clone())
                .on_click(cx.listener(move |this, _, _, cx| {
                    f(this, i, cx);
                    cx.notify();
                })),
        );
    }
    row
}

// ====================================================================== ドロップダウン

pub fn select(
    id: &str,
    items: Vec<PopItem>,
    current: &str,
    width: f32,
    cx: &mut Context<Shell>,
    f: impl Fn(&mut Shell, usize, &mut Window, &mut Context<Shell>) + 'static,
) -> Stateful<Div> {
    let p = theme::pal();
    let anchor = format!("sel:{id}");
    let a2 = anchor.clone();
    let f = Rc::new(f);
    div()
        .id(SharedString::from(format!("select-{id}")))
        .relative()
        .h(px(32.))
        .w(px(width))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(10.))
        .cursor_pointer()
        .child(motion::track(anchor))
        .child(glass::surface_hover(
            glass::Kind::Frost,
            10.,
            format!("sel:{id}"),
        ))
        .child(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .text_size(px(13.))
                .text_color(p.fg)
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(current.to_string()),
        )
        .child(
            div()
                .relative()
                .child(icon("chevrons-up-down", 13., p.faint)),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            let f = f.clone();
            this.open_popup(
                &a2,
                items.clone(),
                width,
                Rc::new(move |s, i, w, cx| f(s, i, w, cx)),
                cx,
            );
        }))
}

// ====================================================================== ステッパー

pub fn stepper(
    id: &str,
    value: i64,
    min: i64,
    max: i64,
    unit: &str,
    cx: &mut Context<Shell>,
    f: impl Fn(&mut Shell, i64, &mut Context<Shell>) + 'static,
) -> Div {
    let p = theme::pal();
    let f = Rc::new(f);
    let f2 = f.clone();
    let btn = |sid: String, ic: &str, enabled: bool| {
        div()
            .id(SharedString::from(sid))
            .size(px(28.))
            .rounded(px(9.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .when(enabled, |d| d.hover(|s| s.bg(p.ctrl_hover)))
            .child(icon(ic, 14., if enabled { p.fg } else { p.faint }))
    };
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(11.))
        .bg(p.ctrl)
        .border_1()
        .border_color(p.hair)
        .child(
            btn(format!("st-{id}-m"), "minus", value > min).on_click(cx.listener(
                move |this, _, _, cx| {
                    if value > min {
                        f(this, value - 1, cx);
                        cx.notify();
                    }
                },
            )),
        )
        .child(
            div()
                .min_w(px(48.))
                .flex()
                .justify_center()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("{value}{unit}")),
        )
        .child(
            btn(format!("st-{id}-p"), "plus", value < max).on_click(cx.listener(
                move |this, _, _, cx| {
                    if value < max {
                        f2(this, value + 1, cx);
                        cx.notify();
                    }
                },
            )),
        )
}

/// 表示を自由にできるステッパー。
pub fn stepper_custom(
    id: &str,
    text: String,
    can_dec: bool,
    can_inc: bool,
    cx: &mut Context<Shell>,
    dec: impl Fn(&mut Shell, &mut Context<Shell>) + 'static,
    inc: impl Fn(&mut Shell, &mut Context<Shell>) + 'static,
) -> Div {
    let p = theme::pal();
    let btn = |sid: String, ic: &str, enabled: bool| {
        div()
            .id(SharedString::from(sid))
            .size(px(26.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .when(enabled, |d| d.hover(|s| s.bg(p.ctrl_hover)))
            .child(icon(ic, 13., if enabled { p.fg } else { p.faint }))
    };
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(10.))
        .bg(p.ctrl)
        .border_1()
        .border_color(p.hair)
        .child(
            btn(format!("sc-{id}-m"), "minus", can_dec).on_click(cx.listener(
                move |this, _, _, cx| {
                    if can_dec {
                        dec(this, cx);
                        cx.notify();
                    }
                },
            )),
        )
        .child(
            div()
                .min_w(px(58.))
                .flex()
                .justify_center()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child(text),
        )
        .child(
            btn(format!("sc-{id}-p"), "plus", can_inc).on_click(cx.listener(
                move |this, _, _, cx| {
                    if can_inc {
                        inc(this, cx);
                        cx.notify();
                    }
                },
            )),
        )
}

// ====================================================================== 浮いたタブバー

pub fn tab_bar(
    id: &str,
    tabs: &[(String, &'static str)],
    selected: usize,
    cx: &mut Context<Shell>,
    f: impl Fn(&mut Shell, usize, &mut Context<Shell>) + 'static,
) -> Div {
    let p = theme::pal();
    let f = Rc::new(f);
    // 各タブの位置を記録し、選択面をばねで動かす。
    let cont = format!("tabs:{id}");
    let sel_rect = motion::rect(&format!("tab:{id}:{selected}"));
    let base = motion::rect(&cont);
    let (tx, tw) = match (sel_rect, base) {
        (Some(r), Some(b)) => (r[0] - b[0], r[2]),
        _ => (0., 0.),
    };
    let x = motion::spring_with(&format!("tabx:{id}"), tx, 20.);
    let w = motion::spring_with(&format!("tabw:{id}"), tw, 20.);
    let mut bar = div()
        .relative()
        .flex()
        .flex_row()
        .p(px(5.))
        .gap(px(2.))
        .rounded(px(18.))
        .shadow(vec![BoxShadow {
            color: theme::alpha(p.shadow, p.shadow.a * 0.7),
            offset: point(px(0.), px(8.)),
            blur_radius: px(24.),
            spread_radius: px(-10.),
            inset: false,
        }])
        .child(glass::surface(glass::Kind::Clear, 18.))
        .child(motion::track(cont));
    if w > 1. {
        bar = bar.child(
            div()
                .absolute()
                .top(px(5.))
                .bottom(px(5.))
                .left(px(x))
                .w(px(w))
                .rounded(px(13.))
                .bg(if p.light {
                    theme::hex(0xffffff, 0.9)
                } else {
                    theme::hex(0xffffff, 0.14)
                })
                .border_1()
                .border_color(p.edge)
                .shadow(vec![BoxShadow {
                    color: theme::hex(0x000000, if p.light { 0.1 } else { 0.35 }),
                    offset: point(px(0.), px(3.)),
                    blur_radius: px(10.),
                    spread_radius: px(-3.),
                    inset: false,
                }]),
        );
    }
    for (i, (label, ic)) in tabs.iter().enumerate() {
        let on = i == selected;
        let f = f.clone();
        bar = bar.child(
            div()
                .id(SharedString::from(format!("tab-{id}-{i}")))
                .relative()
                .h(px(34.))
                .px(px(14.))
                .flex()
                .items_center()
                .gap(px(7.))
                .rounded(px(13.))
                .cursor_pointer()
                .when(on && w <= 1., |d| d.bg(p.ctrl_hover))
                .when(!on, |d| d.hover(|s| s.bg(p.ctrl)))
                .child(motion::track(format!("tab:{id}:{i}")))
                .child(icon(ic, 15., if on { p.accent } else { p.sub }))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(if on {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if on { p.fg } else { p.sub })
                        .whitespace_nowrap()
                        .child(label.clone()),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    f(this, i, cx);
                    cx.notify();
                })),
        );
    }
    bar
}

/// 横に並ぶカードのグリッド (幅に応じて 1〜2 列)。
pub fn grid2(children: Vec<AnyElement>) -> Div {
    let mut left = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(theme::GAP));
    let mut right = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(theme::GAP));
    for (i, c) in children.into_iter().enumerate() {
        if i % 2 == 0 {
            left = left.child(c);
        } else {
            right = right.child(c);
        }
    }
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(theme::GAP))
        .child(left)
        .child(right)
}

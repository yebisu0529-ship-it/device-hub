//! 最前面の層: ポップアップメニュー・モーダル・トースト。

use crate::app::{Modal, Shell};
use crate::ui::{self, Btn};
use crate::{glass, keycode, motion, theme};
use devhub_core::keys::{self, MEDIA_FNS, MOUSE_FNS};
use devhub_core::*;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

const PRESETS: &[u32] = &[
    0xff3b4f, 0xff7a3d, 0xffc53d, 0xc8f04a, 0x3ddc84, 0x2ee6c8, 0x3ec7ff, 0x3d7bff, 0x7a5cff,
    0xc35cff, 0xff5cc8, 0xffffff, 0xb0b8c8, 0x6a7385,
];

impl Shell {
    // ------------------------------------------------------------------ ポップアップ

    pub fn popup_layer(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pop = self.popup.as_ref()?;
        let p = theme::pal();
        let k = motion::spring_with("popup-in", 1., 26.);
        let [ax, ay, aw, ah] = pop.anchor;
        let w = aw.max(pop.min_w).max(160.);
        let items = pop.items.clone();
        let n = items.len();
        let item_h = 34.;
        let height = (n as f32 * item_h + 12.).min(380.);
        // 画面下にはみ出すなら上に開く。
        let (_, win_h) = motion::rect("root-size")
            .map(|r| (r[2], r[3]))
            .unwrap_or((1280., 800.));
        let below = ay + ah + 6. + height < win_h - 8.;
        let top = if below {
            ay + ah + 6.
        } else {
            (ay - 6. - height).max(8.)
        };
        let mut list = div().relative().flex().flex_col().p(px(6.));
        for (i, it) in items.into_iter().enumerate() {
            let sel = it.selected;
            list = list.child(
                div()
                    .id(SharedString::from(format!("pop-{i}")))
                    .h(px(item_h))
                    .px(px(10.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(9.))
                    .rounded(px(9.))
                    .cursor_pointer()
                    .hover(|s| s.bg(p.ctrl_hover))
                    .when(sel, |d| d.bg(p.accent_soft))
                    .when_some(it.swatch, |d, c| d.child(ui::swatch(c, 14., false)))
                    .when_some(it.icon, |d, ic| d.child(ui::icon(ic, 14., p.sub)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .text_color(if it.danger { p.danger } else { p.fg })
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(it.label.clone()),
                    )
                    .when_some(it.sub.clone(), |d, s| d.child(ui::faint(s)))
                    .when(sel, |d| d.child(ui::icon("check", 14., p.accent)))
                    .on_click(cx.listener(move |this, _, w, cx| {
                        if let Some(pop) = this.popup.take() {
                            (pop.on_pick)(this, i, w, cx);
                        }
                        cx.notify();
                    })),
            );
        }
        Some(
            div()
                .id("popup-scrim")
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.popup = None;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .id("popup")
                        .absolute()
                        .left(px(ax))
                        .top(px(top + (1. - k) * if below { -6. } else { 6. }))
                        .w(px(w))
                        .max_h(px(380.))
                        .opacity(k)
                        .rounded(px(14.))
                        .shadow(vec![BoxShadow {
                            color: p.shadow,
                            offset: point(px(0.), px(16.)),
                            blur_radius: px(40.),
                            spread_radius: px(-8.),
                            inset: false,
                        }])
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(glass::surface(glass::Kind::Sheet, 14.))
                        .child(
                            div()
                                .id("popup-list")
                                .relative()
                                .max_h(px(380.))
                                .overflow_y_scroll()
                                .child(list),
                        ),
                )
                .into_any_element(),
        )
    }

    // ------------------------------------------------------------------ トースト

    pub fn toast_layer(&mut self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.toasts.is_empty() {
            return None;
        }
        let p = theme::pal();
        let mut col = div()
            .absolute()
            .bottom(px(24.))
            .right(px(24.))
            .flex()
            .flex_col()
            .items_end()
            .gap(px(8.));
        for (i, t) in self.toasts.iter().enumerate() {
            let k = motion::spring_with(&format!("toast:{:?}", t.born), 1., 22.);
            let c = if t.error { p.danger } else { p.ok };
            col = col.child(
                div()
                    .relative()
                    .max_w(px(420.))
                    .px(px(14.))
                    .py(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(14.))
                    .opacity(k)
                    .ml(px((1. - k) * 30.))
                    .shadow(vec![BoxShadow {
                        color: p.shadow,
                        offset: point(px(0.), px(10.)),
                        blur_radius: px(30.),
                        spread_radius: px(-8.),
                        inset: false,
                    }])
                    .child(glass::surface(glass::Kind::Sheet, 14.))
                    .child(div().relative().child(ui::icon(
                        if t.error {
                            "circle-alert"
                        } else {
                            "circle-check"
                        },
                        16.,
                        c,
                    )))
                    .child(
                        div()
                            .relative()
                            .text_size(px(13.))
                            .text_color(p.fg)
                            .child(t.msg.clone()),
                    )
                    .id(SharedString::from(format!("toast-{i}"))),
            );
        }
        Some(col.into_any_element())
    }

    // ------------------------------------------------------------------ モーダル

    pub fn modal_layer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.modal.as_ref()?;
        let p = theme::pal();
        let k = motion::spring_with("modal-in", 1., 24.);
        let (title, body, width): (String, AnyElement, f32) = match self.modal.as_ref()? {
            Modal::Confirm { title, .. } => (title.clone(), self.modal_confirm(cx), 420.),
            Modal::Button { slot, .. } => {
                let name = self
                    .cur()
                    .and_then(|s| s.caps.as_ref())
                    .and_then(|c| c.mouse.as_ref())
                    .and_then(|m| m.buttons.get(*slot))
                    .map(|b| b.name.clone())
                    .unwrap_or_default();
                (
                    format!("{name} の割り当て"),
                    self.modal_button(window, cx),
                    620.,
                )
            }
            Modal::CaptureKey { title, .. } => (title.clone(), self.modal_capture(cx), 420.),
            Modal::Color { title, .. } => (title.clone(), self.modal_color(window, cx), 440.),
            Modal::Text { title, .. } => (title.clone(), self.modal_text(window, cx), 420.),
            Modal::Palette => ("コマンド".into(), self.modal_palette(window, cx), 560.),
            Modal::Calibrate { .. } => (
                "磁軸キャリブレーション".into(),
                self.modal_calibrate(window, cx),
                900.,
            ),
        };
        Some(
            div()
                .id("modal-scrim")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::alpha(p.scrim, p.scrim.a * k))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        if this.popup.is_none() {
                            this.modal = None;
                            cx.notify();
                        }
                    }),
                )
                .child(
                    div()
                        .id("modal")
                        .relative()
                        .w(px(width))
                        .max_h(relative(0.9))
                        .mt(px((1. - k) * 24.))
                        .opacity(k)
                        .rounded(px(24.))
                        .shadow(vec![BoxShadow {
                            color: p.shadow,
                            offset: point(px(0.), px(24.)),
                            blur_radius: px(60.),
                            spread_radius: px(-10.),
                            inset: false,
                        }])
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(glass::surface(glass::Kind::Sheet, 24.))
                        .child(
                            div()
                                .relative()
                                .flex()
                                .flex_col()
                                .gap(px(14.))
                                .p(px(22.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_size(px(17.))
                                                .font_weight(FontWeight::BOLD)
                                                .child(title),
                                        )
                                        .child(ui::icon_button(
                                            "modal-x",
                                            "x",
                                            cx,
                                            |this, _, cx| {
                                                this.modal = None;
                                                cx.notify();
                                            },
                                        )),
                                )
                                .child(body),
                        ),
                )
                .into_any_element(),
        )
    }

    fn modal_footer(
        &mut self,
        ok_label: &str,
        danger: bool,
        cx: &mut Context<Self>,
        ok: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
    ) -> Div {
        div()
            .flex()
            .justify_end()
            .gap(px(8.))
            .pt(px(4.))
            .child(ui::button(
                "m-cancel",
                "キャンセル",
                None,
                Btn::Ghost,
                cx,
                |this, _, cx| {
                    this.modal = None;
                    cx.notify();
                },
            ))
            .child(ui::button(
                "m-ok",
                ok_label,
                None,
                if danger { Btn::Danger } else { Btn::Primary },
                cx,
                ok,
            ))
    }

    fn modal_confirm(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(Modal::Confirm {
            body, ok, danger, ..
        }) = self.modal.as_ref()
        else {
            return div().into_any_element();
        };
        let (body, ok, danger) = (body.clone(), ok.clone(), *danger);
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(ui::sub(body))
            .child(self.modal_footer(&ok, danger, cx, |this, w, cx| {
                if let Some(Modal::Confirm { on_ok, .. }) = this.modal.take() {
                    on_ok(this, w, cx);
                }
                cx.notify();
            }))
            .into_any_element()
    }

    fn modal_text(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let input = self.input("modal", "", "", window, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(Input::new(&input))
            .child(self.modal_footer("保存", false, cx, |this, w, cx| {
                let v = this.input_value("modal", cx).unwrap_or_default();
                if let Some(Modal::Text { on_ok, .. }) = this.modal.take() {
                    on_ok(this, v, w, cx);
                }
                cx.notify();
            }))
            .into_any_element()
    }

    fn modal_capture(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = theme::pal();
        let Some(Modal::CaptureKey { got, .. }) = self.modal.as_ref() else {
            return div().into_any_element();
        };
        let label = got
            .map(keycode::stroke_label)
            .unwrap_or_else(|| "キーを押してください…".into());
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .h(px(84.))
                    .rounded(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(p.ctrl)
                    .border_1()
                    .border_color(theme::alpha(p.accent, 0.4))
                    .child(ui::big_number(label, 26.)),
            )
            .child(ui::faint(
                "修飾キー (Ctrl / Shift / Alt / Win) と組み合わせて押せます",
            ))
            .child(self.modal_footer("決定", false, cx, |this, w, cx| {
                if let Some(Modal::CaptureKey {
                    got: Some(k),
                    on_key,
                    ..
                }) = this.modal.take()
                {
                    on_key(this, k, w, cx);
                }
                cx.notify();
            }))
            .into_any_element()
    }

    // ------------------------------------------------------------------ 色

    fn modal_color(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = theme::pal();
        let Some(Modal::Color { draft, .. }) = self.modal.as_ref() else {
            return div().into_any_element();
        };
        let c = *draft;
        let (h, s, v) = rgb_to_hsv(c);
        let mut presets = div().flex().flex_wrap().gap(px(8.));
        for (i, hx) in PRESETS.iter().enumerate() {
            let rc = Rgb::hex(*hx);
            presets = presets.child(
                div()
                    .id(SharedString::from(format!("preset-{i}")))
                    .cursor_pointer()
                    .child(ui::swatch(theme::rgb_of(rc), 26., rc == c))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        set_color_draft(this, rc);
                        cx.notify();
                    })),
            );
        }
        let hex_in = self.input("hex", &format!("{:06X}", c.to_hex()), "RRGGBB", window, cx);
        let hue_bar = div()
            .absolute()
            .left(px(11.))
            .right(px(11.))
            .top(px(12.))
            .h(px(6.))
            .rounded(px(3.))
            .bg(linear_gradient(
                90.,
                linear_color_stop(hsla(0., 1., 0.5, 1.), 0.),
                linear_color_stop(hsla(0.999, 1., 0.5, 1.), 1.),
            ));
        let _ = hue_bar;
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(
                        div()
                            .size(px(64.))
                            .rounded(px(18.))
                            .bg(theme::rgb_of(c))
                            .border_1()
                            .border_color(p.edge)
                            .shadow(vec![BoxShadow {
                                color: theme::alpha(theme::rgb_of(c), 0.5),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(24.),
                                spread_radius: px(-6.),
                                inset: false,
                            }]),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(ui::faint("16 進"))
                            .child(Input::new(&hex_in))
                            .child(ui::button(
                                "hex-apply",
                                "反映",
                                None,
                                Btn::Glass,
                                cx,
                                |this, _, cx| {
                                    if let Some(v) = this.input_value("hex", cx) {
                                        if let Ok(n) = u32::from_str_radix(
                                            v.trim().trim_start_matches('#'),
                                            16,
                                        ) {
                                            set_color_draft(this, Rgb::hex(n & 0xffffff));
                                            cx.notify();
                                        }
                                    }
                                },
                            )),
                    ),
            )
            .child(presets)
            .child(ui::row_first(
                "色相",
                None,
                ui::slider(
                    "hue",
                    h,
                    0.,
                    359.,
                    1.,
                    Some(theme::rgb_of(hsv_to_rgb(h, 1., 1.))),
                    cx,
                    move |this, nh, cx| {
                        set_color_draft(this, hsv_to_rgb(nh, s.max(0.05), v.max(0.05)));
                        cx.notify();
                    },
                )
                .w(px(260.)),
            ))
            .child(ui::row(
                "彩度",
                None,
                ui::slider("sat", s, 0., 1., 0.01, None, cx, move |this, ns, cx| {
                    set_color_draft(this, hsv_to_rgb(h, ns, v.max(0.05)));
                    cx.notify();
                })
                .w(px(260.)),
            ))
            .child(ui::row(
                "明るさ",
                None,
                ui::slider("val", v, 0., 1., 0.01, None, cx, move |this, nv, cx| {
                    set_color_draft(this, hsv_to_rgb(h, s, nv));
                    cx.notify();
                })
                .w(px(260.)),
            ))
            .child(self.modal_footer("決定", false, cx, |this, w, cx| {
                if let Some(Modal::Color { draft, on_ok, .. }) = this.modal.take() {
                    on_ok(this, draft, w, cx);
                }
                cx.notify();
            }))
            .into_any_element()
    }

    // ------------------------------------------------------------------ マウスボタン割り当て

    fn modal_button(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = theme::pal();
        let Some(Modal::Button { draft, capture, .. }) = self.modal.as_ref() else {
            return div().into_any_element();
        };
        let (draft, capture) = (draft.clone(), *capture);
        let Some(mc) = self
            .cur()
            .and_then(|s| s.caps.as_ref())
            .and_then(|c| c.mouse.clone())
        else {
            return div().into_any_element();
        };
        let macros: Vec<Option<MacroDef>> = self
            .cur()
            .and_then(|s| s.state.as_ref())
            .and_then(|s| s.mouse.as_ref())
            .map(|m| m.macros.clone())
            .unwrap_or_default();
        let kinds: Vec<ButtonActionKind> = mc.button_kinds.clone();
        let cur_kind = draft.kind();
        let kind_label = |k: ButtonActionKind| match k {
            ButtonActionKind::Disabled => "無効",
            ButtonActionKind::Mouse => "マウス",
            ButtonActionKind::Dpi => "DPI",
            ButtonActionKind::Fire => "連射",
            ButtonActionKind::Key => "キーボード",
            ButtonActionKind::Media => "メディア",
            ButtonActionKind::Macro => "マクロ",
            ButtonActionKind::PollingCycle => "ポーリング切替",
            ButtonActionKind::ProfileCycle => "プロファイル切替",
            ButtonActionKind::ScrollModeToggle => "上下ロール切替",
        };
        // 種類のチップ。
        let mut kinds_row = div().flex().flex_wrap().gap(px(6.));
        for k in kinds.iter().copied() {
            let on = k == cur_kind;
            kinds_row = kinds_row.child(
                div()
                    .id(SharedString::from(format!("bk-{k:?}")))
                    .h(px(30.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .rounded(px(10.))
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .border_1()
                    .when(on, |d| {
                        d.bg(p.accent_soft)
                            .border_color(theme::alpha(p.accent, 0.5))
                            .text_color(p.fg)
                    })
                    .when(!on, |d| {
                        d.bg(p.ctrl)
                            .border_color(p.hair)
                            .text_color(p.sub)
                            .hover(|s| s.bg(p.ctrl_hover))
                    })
                    .child(kind_label(k))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let nd = default_action(k);
                        if let Some(Modal::Button { draft, capture, .. }) = this.modal.as_mut() {
                            if draft.kind() != k {
                                *draft = nd.clone();
                            }
                            *capture = false;
                        }
                        cx.notify();
                    })),
            );
        }

        let grid_btn = |id: String, label: &str, on: bool| {
            div()
                .id(SharedString::from(id))
                .h(px(36.))
                .px(px(12.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(11.))
                .cursor_pointer()
                .text_size(px(12.5))
                .border_1()
                .when(on, |d| {
                    d.bg(p.accent_soft)
                        .border_color(theme::alpha(p.accent, 0.55))
                        .text_color(p.fg)
                })
                .when(!on, |d| {
                    d.bg(p.ctrl)
                        .border_color(p.hair)
                        .text_color(p.sub)
                        .hover(|s| s.bg(p.ctrl_hover))
                })
                .child(label.to_string())
        };
        let set = |a: ButtonAction| {
            move |this: &mut Shell, _: &ClickEvent, _: &mut Window, cx: &mut Context<Shell>| {
                if let Some(Modal::Button { draft, .. }) = this.modal.as_mut() {
                    *draft = a.clone();
                }
                cx.notify();
            }
        };

        let allows = |action: &ButtonAction| {
            mc.button_options
                .as_ref()
                .is_none_or(|options| options.contains(action))
        };
        let detail: AnyElement = match &draft {
            ButtonAction::Mouse(m) => {
                let mut g = div().grid().grid_cols(4).gap(px(6.));
                for f in MOUSE_FNS.iter().filter(|f| allows(&ButtonAction::Mouse(**f))) {
                    g = g.child(
                        grid_btn(format!("mf-{f:?}"), keys::mouse_fn_name(*f), *m == *f)
                            .on_click(cx.listener(set(ButtonAction::Mouse(*f)))),
                    );
                }
                g.into_any_element()
            }
            ButtonAction::Media(m) => {
                let mut g = div().grid().grid_cols(3).gap(px(6.));
                for f in MEDIA_FNS.iter().filter(|f| allows(&ButtonAction::Media(**f))) {
                    g = g.child(
                        grid_btn(format!("md-{f:?}"), keys::media_fn_name(*f), *m == *f)
                            .on_click(cx.listener(set(ButtonAction::Media(*f)))),
                    );
                }
                g.into_any_element()
            }
            ButtonAction::Dpi(f) => {
                let shift = if let DpiFn::Shift(v) = f {
                    Some(*v)
                } else {
                    None
                };
                let mut g = div().grid().grid_cols(4).gap(px(6.));
                for (lab, a) in [
                    ("DPI 循環", DpiFn::Cycle),
                    ("DPI +", DpiFn::Up),
                    ("DPI −", DpiFn::Down),
                    (
                        "DPI ロック (押している間)",
                        DpiFn::Shift(shift.unwrap_or(400)),
                    ),
                ] {
                    if !allows(&ButtonAction::Dpi(a)) {
                        continue;
                    }
                    let on = std::mem::discriminant(f) == std::mem::discriminant(&a);
                    g = g.child(
                        grid_btn(format!("dp-{lab}"), lab, on)
                            .on_click(cx.listener(set(ButtonAction::Dpi(a)))),
                    );
                }
                let (min, max, step) = (mc.dpi_min as f32, mc.dpi_max as f32, mc.dpi_step as f32);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(g)
                    .when_some(shift, |d, v| {
                        d.child(ui::row(
                            "押している間の DPI",
                            None,
                            ui::slider_value(
                                "dpishift",
                                v as f32,
                                min,
                                max.min(6400.),
                                step,
                                "",
                                cx,
                                |this, nv, cx| {
                                    if let Some(Modal::Button { draft, .. }) = this.modal.as_mut() {
                                        *draft = ButtonAction::Dpi(DpiFn::Shift(nv as u32));
                                    }
                                    cx.notify();
                                },
                            ),
                        ))
                    })
                    .into_any_element()
            }
            ButtonAction::Fire {
                button,
                count,
                interval_ms,
            } => {
                let (b, c, i) = (*button, *count, *interval_ms);
                div()
                    .flex()
                    .flex_col()
                    .child(ui::row_first(
                        "連射するボタン",
                        None,
                        ui::segmented(
                            "fire-btn",
                            &[
                                "左クリック".into(),
                                "右クリック".into(),
                                "中クリック".into(),
                            ],
                            match b {
                                MouseFn::Right => 1,
                                MouseFn::Middle => 2,
                                _ => 0,
                            },
                            cx,
                            move |this, ix, cx| {
                                let nb = [MouseFn::Left, MouseFn::Right, MouseFn::Middle][ix];
                                if let Some(Modal::Button { draft, .. }) = this.modal.as_mut() {
                                    *draft = ButtonAction::Fire {
                                        button: nb,
                                        count: c,
                                        interval_ms: i,
                                    };
                                }
                                cx.notify();
                            },
                        )
                        .w(px(300.)),
                    ))
                    .child(ui::row(
                        "回数",
                        None,
                        ui::stepper(
                            "fire-count",
                            c as i64,
                            1,
                            255,
                            "回",
                            cx,
                            move |this, v, cx| {
                                if let Some(Modal::Button { draft, .. }) = this.modal.as_mut() {
                                    *draft = ButtonAction::Fire {
                                        button: b,
                                        count: v as u8,
                                        interval_ms: i,
                                    };
                                }
                                cx.notify();
                            },
                        ),
                    ))
                    .child(ui::row(
                        "間隔",
                        None,
                        ui::slider_value(
                            "fire-int",
                            i as f32,
                            10.,
                            255.,
                            1.,
                            "ms",
                            cx,
                            move |this, v, cx| {
                                if let Some(Modal::Button { draft, .. }) = this.modal.as_mut() {
                                    *draft = ButtonAction::Fire {
                                        button: b,
                                        count: c,
                                        interval_ms: v as u16,
                                    };
                                }
                                cx.notify();
                            },
                        ),
                    ))
                    .into_any_element()
            }
            ButtonAction::Key(stroke) => {
                let s = *stroke;
                let mut mods_row = div().flex().gap(px(6.));
                for (lab, bit) in [
                    ("Ctrl", mods::LCTRL),
                    ("Shift", mods::LSHIFT),
                    ("Alt", mods::LALT),
                    ("Win", mods::LGUI),
                ] {
                    let on = s.mods & bit != 0;
                    mods_row = mods_row.child(
                        grid_btn(format!("mod-{lab}"), lab, on)
                            .w(px(64.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(Modal::Button {
                                    draft: ButtonAction::Key(k),
                                    ..
                                }) = this.modal.as_mut()
                                {
                                    k.mods ^= bit;
                                }
                                cx.notify();
                            })),
                    );
                }
                let items: Vec<crate::app::PopItem> = keys::HID_KEYS
                    .iter()
                    .map(|k| crate::app::PopItem::new(k.1, k.0 == s.usage))
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .id("cap-box")
                            .h(px(70.))
                            .rounded(px(16.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px(2.))
                            .cursor_pointer()
                            .bg(p.ctrl)
                            .border_1()
                            .border_color(if capture { p.accent } else { p.hair })
                            .child(ui::big_number(
                                if capture {
                                    "キーを押してください…".to_string()
                                } else {
                                    keycode::stroke_label(s)
                                },
                                22.,
                            ))
                            .child(ui::faint(if capture {
                                "Esc 以外の任意のキー"
                            } else {
                                "クリックしてキーを取り込む"
                            }))
                            .on_click(cx.listener(|this, _, w, cx| {
                                if let Some(Modal::Button { capture, .. }) = this.modal.as_mut() {
                                    *capture = true;
                                }
                                w.focus(&this.focus, cx);
                                cx.notify();
                            })),
                    )
                    .child(ui::row_first("修飾キー", None, mods_row))
                    .child(ui::row(
                        "一覧から選ぶ",
                        None,
                        ui::select(
                            "btn-key",
                            items,
                            &keys::hid_name(s.usage),
                            180.,
                            cx,
                            move |this, i, _, cx| {
                                let u = keys::HID_KEYS[i].0;
                                if let Some(Modal::Button {
                                    draft: ButtonAction::Key(k),
                                    ..
                                }) = this.modal.as_mut()
                                {
                                    k.usage = u;
                                }
                                cx.notify();
                            },
                        ),
                    ))
                    .into_any_element()
            }
            ButtonAction::Macro { slot, run } => {
                let (slot, run) = (*slot, *run);
                let items: Vec<crate::app::PopItem> = macros
                    .iter()
                    .enumerate()
                    .map(|(i, m)| {
                        crate::app::PopItem::new(
                            m.as_ref()
                                .map(|m| format!("{}: {}", i + 1, m.name))
                                .unwrap_or_else(|| format!("{}: (空)", i + 1)),
                            i as u8 == slot,
                        )
                    })
                    .collect();
                let cur = macros
                    .get(slot as usize)
                    .and_then(|m| m.as_ref())
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| format!("スロット {}", slot + 1));
                let times = if let MacroRun::Times(n) = run { n } else { 1 };
                div()
                    .flex()
                    .flex_col()
                    .child(ui::row_first(
                        "マクロ",
                        if macros.iter().all(|m| m.is_none()) {
                            Some("先に「マクロ」タブで作成してください")
                        } else {
                            None
                        },
                        ui::select("btn-macro", items, &cur, 220., cx, move |this, i, _, cx| {
                            if let Some(Modal::Button {
                                draft: ButtonAction::Macro { slot, .. },
                                ..
                            }) = this.modal.as_mut()
                            {
                                *slot = i as u8;
                            }
                            cx.notify();
                        }),
                    ))
                    .child(ui::row(
                        "実行方法",
                        None,
                        ui::segmented(
                            "btn-macro-run",
                            &["回数指定".into(), "離すまで".into(), "再度押すまで".into()],
                            match run {
                                MacroRun::Times(_) => 0,
                                MacroRun::WhileHeld => 1,
                                MacroRun::Toggle => 2,
                            },
                            cx,
                            move |this, i, cx| {
                                let r = [
                                    MacroRun::Times(times),
                                    MacroRun::WhileHeld,
                                    MacroRun::Toggle,
                                ][i];
                                if let Some(Modal::Button {
                                    draft: ButtonAction::Macro { run, .. },
                                    ..
                                }) = this.modal.as_mut()
                                {
                                    *run = r;
                                }
                                cx.notify();
                            },
                        )
                        .w(px(320.)),
                    ))
                    .when(matches!(run, MacroRun::Times(_)), |d| {
                        d.child(ui::row(
                            "回数",
                            None,
                            ui::stepper(
                                "btn-macro-times",
                                times as i64,
                                1,
                                255,
                                "回",
                                cx,
                                |this, v, cx| {
                                    if let Some(Modal::Button {
                                        draft: ButtonAction::Macro { run, .. },
                                        ..
                                    }) = this.modal.as_mut()
                                    {
                                        *run = MacroRun::Times(v as u16);
                                    }
                                    cx.notify();
                                },
                            ),
                        ))
                    })
                    .into_any_element()
            }
            ButtonAction::Disabled => {
                ui::sub("このボタンを押しても何も起きません。").into_any_element()
            }
            ButtonAction::PollingCycle => {
                ui::sub("押すたびにポーリングレートを順に切り替えます。").into_any_element()
            }
            ButtonAction::ProfileCycle => {
                ui::sub("押すたびにオンボードプロファイルを順に切り替えます。").into_any_element()
            }
            ButtonAction::ScrollModeToggle => {
                ui::sub("ホイールの上下ロール動作を切り替えます。").into_any_element()
            }
            ButtonAction::Unsupported { .. } => {
                ui::sub("このデバイスに記録されている割当は未対応の種類です。別の割当を選ぶと元には戻せません。")
                    .into_any_element()
            }
        };
        let _ = window;
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(kinds_row)
            .child(div().min_h(px(110.)).child(detail))
            .child(self.modal_footer("適用", false, cx, |this, _, cx| {
                let Some(Modal::Button { slot, draft, .. }) = this.modal.take() else {
                    return;
                };
                // 左クリックが 1 つも無くなる割り当ては拒否する (mouse.fit と同じ)。
                let need_left = this
                    .cur()
                    .and_then(|s| s.caps.as_ref())
                    .and_then(|c| c.mouse.as_ref())
                    .map(|m| m.require_left_click)
                    .unwrap_or(false);
                if need_left {
                    let others_left =
                        this.cur()
                            .and_then(|s| s.state.as_ref())
                            .and_then(|s| s.mouse.as_ref())
                            .map(|m| {
                                m.buttons.iter().enumerate().any(|(i, b)| {
                                    i != slot && *b == ButtonAction::Mouse(MouseFn::Left)
                                })
                            })
                            .unwrap_or(true);
                    if !others_left && draft != ButtonAction::Mouse(MouseFn::Left) {
                        this.toast(
                            "少なくとも 1 つのボタンを左クリックにしてください",
                            true,
                            cx,
                        );
                        this.modal = Some(Modal::Button {
                            slot,
                            draft,
                            capture: false,
                        });
                        return;
                    }
                }
                this.apply(
                    Change::Button {
                        slot,
                        action: draft,
                    },
                    cx,
                );
            }))
            .into_any_element()
    }
}

fn default_action(k: ButtonActionKind) -> ButtonAction {
    match k {
        ButtonActionKind::Disabled => ButtonAction::Disabled,
        ButtonActionKind::Mouse => ButtonAction::Mouse(MouseFn::Left),
        ButtonActionKind::Dpi => ButtonAction::Dpi(DpiFn::Cycle),
        ButtonActionKind::Fire => ButtonAction::Fire {
            button: MouseFn::Left,
            count: 3,
            interval_ms: 40,
        },
        ButtonActionKind::Key => ButtonAction::Key(KeyStroke {
            mods: 0,
            usage: 0x04,
        }),
        ButtonActionKind::Media => ButtonAction::Media(MediaFn::PlayPause),
        ButtonActionKind::Macro => ButtonAction::Macro {
            slot: 0,
            run: MacroRun::Times(1),
        },
        ButtonActionKind::PollingCycle => ButtonAction::PollingCycle,
        ButtonActionKind::ProfileCycle => ButtonAction::ProfileCycle,
        ButtonActionKind::ScrollModeToggle => ButtonAction::ScrollModeToggle,
    }
}

fn set_color_draft(this: &mut Shell, c: Rgb) {
    if let Some(Modal::Color { draft, .. }) = this.modal.as_mut() {
        *draft = c;
    }
}

pub fn rgb_to_hsv(c: Rgb) -> (f32, f32, f32) {
    let (r, g, b) = (c.r as f32 / 255., c.g as f32 / 255., c.b as f32 / 255.);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d < 1e-5 {
        0.
    } else if max == r {
        60. * (((g - b) / d).rem_euclid(6.))
    } else if max == g {
        60. * ((b - r) / d + 2.)
    } else {
        60. * ((r - g) / d + 4.)
    };
    let s = if max <= 0. { 0. } else { d / max };
    (h, s, max)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    let c = v * s;
    let x = c * (1. - ((h / 60.).rem_euclid(2.) - 1.).abs());
    let m = v - c;
    let (r, g, b) = match (h.rem_euclid(360.) / 60.) as i32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    Rgb::new(
        ((r + m) * 255.).round() as u8,
        ((g + m) * 255.).round() as u8,
        ((b + m) * 255.).round() as u8,
    )
}

/// 色選択ダイアログを開く。
pub fn pick_color(
    this: &mut Shell,
    title: &str,
    current: Rgb,
    window: &mut Window,
    cx: &mut Context<Shell>,
    on_ok: impl Fn(&mut Shell, Rgb, &mut Window, &mut Context<Shell>) + 'static,
) {
    this.modal = Some(Modal::Color {
        title: title.into(),
        draft: current,
        on_ok: Rc::new(on_ok),
    });
    this.set_input("hex", &format!("{:06X}", current.to_hex()), window, cx);
    cx.notify();
}

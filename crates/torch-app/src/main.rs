//! Torch desktop app. Thin GPUI layer over `torch-core`.

use gpui::{
    App, Bounds, Context, SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};

/// Root view: for now just shows the product name.
#[derive(Debug)]
struct TorchRoot {
    title: SharedString,
}

impl Render for TorchRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .justify_center()
            .items_center()
            .bg(rgb(0x1e1e22))
            .text_color(rgb(0xf0f0f0))
            .text_3xl()
            .child(self.title.clone())
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.), px(640.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(torch_core::PRODUCT_NAME.into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        // Failing to open the main window at startup is unrecoverable, so a panic with a
        // clear message is the right outcome here.
        cx.open_window(options, |_, cx| {
            cx.new(|_| TorchRoot {
                title: torch_core::PRODUCT_NAME.into(),
            })
        })
        .expect("failed to open the main window");
        cx.activate(true);
    });
}

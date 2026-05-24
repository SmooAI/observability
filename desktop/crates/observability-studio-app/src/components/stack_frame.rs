//! Stack-frame renderer. Pulls SDK-shape `frames[]` and lays them out as
//! one card per frame with optional inline source-context lines.

use dioxus::prelude::*;
use observability_studio_client::api::errors::StackFrame;

#[derive(Props, Clone, PartialEq)]
pub struct StackFramesProps {
    pub frames: Vec<StackFrame>,
}

#[component]
pub fn StackFrames(props: StackFramesProps) -> Element {
    rsx! {
        for (i, frame) in props.frames.iter().cloned().enumerate() {
            {
                let key = format!("frame-{i}");
                let function = frame.function.clone().unwrap_or_else(|| "<anonymous>".into());
                let file = frame
                    .filename
                    .clone()
                    .or_else(|| frame.module.clone())
                    .unwrap_or_else(|| "<unknown>".into());
                let line_label = frame
                    .lineno
                    .map(|n| format!(":{n}"))
                    .unwrap_or_default();
                let has_ctx = frame.context_line.is_some()
                    || !frame.pre_context.is_empty()
                    || !frame.post_context.is_empty();
                rsx! {
                    div { key: "{key}", class: "frame",
                        span { class: "frame__sig",
                            "at "
                            span { class: "frame__sig-fn", "{function}" }
                            " ({file}{line_label})"
                        }
                        if has_ctx {
                            div { class: "frame__src",
                                {render_src_lines(&frame)}
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_src_lines(frame: &StackFrame) -> Element {
    let base = frame.lineno.unwrap_or(0);
    let pre_offset = frame.pre_context.len() as i64;
    let start = base - pre_offset;
    let mut line_no = start;
    let mut lines: Vec<(i64, String, bool)> = Vec::new();
    for l in &frame.pre_context {
        lines.push((line_no, l.clone(), false));
        line_no += 1;
    }
    if let Some(ctx) = &frame.context_line {
        lines.push((line_no, ctx.clone(), true));
        line_no += 1;
    }
    for l in &frame.post_context {
        lines.push((line_no, l.clone(), false));
        line_no += 1;
    }
    rsx! {
        for (n, content, hit) in lines.into_iter() {
            {
                let class = if hit { "frame__src-line frame__src-line--hit" } else { "frame__src-line" };
                // 4-wide right-padded line number; mono font keeps columns
                // aligned even without explicit tabular numerals on this
                // element.
                let prefix = format!("{n:>4} ");
                rsx! {
                    span { key: "{n}", class: "{class}", "{prefix}{content}" }
                }
            }
        }
    }
}

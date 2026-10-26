//! Right-to-left text in the native renderer, measured on rendered galleys.
//!
//! These use the Windows fonts loaded by `configure` and the bidirectional layout of
//! the patched epaint in vendor/epaint.
use super::*;
use egui::text::{Galley, LayoutJob, TextFormat};
use std::sync::Arc;

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    configure(&ctx);
    ctx
}

fn galley(ctx: &egui::Context, job: LayoutJob) -> Arc<Galley> {
    let mut galley = None;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        galley = Some(ui.fonts_mut(|fonts| fonts.layout_job(job.clone())));
    });
    output.textures_delta.clear();
    galley.unwrap()
}

fn layout(ctx: &egui::Context, text: &str, wrap_width: f32) -> Arc<Galley> {
    let job = LayoutJob::simple(
        text.to_owned(),
        egui::FontId::proportional(13.0),
        Color32::WHITE,
        wrap_width,
    );
    galley(ctx, job)
}

/// Each row's characters from left to right as drawn. A right-to-left word appears
/// reversed; zero-width glyphs (marks, ligature continuations) are left out.
fn visual_rows(galley: &Galley) -> Vec<String> {
    galley
        .rows
        .iter()
        .map(|row| {
            let glyphs: Vec<_> = row
                .row
                .glyphs
                .iter()
                .filter(|glyph| glyph.advance_width > 0.0)
                .collect();
            assert!(
                glyphs.windows(2).all(|pair| pair[0].pos.x < pair[1].pos.x),
                "Glyphs are stored left to right"
            );
            glyphs.iter().map(|glyph| glyph.chr).collect()
        })
        .collect()
}

fn reversed(text: &str) -> String {
    text.chars().rev().collect()
}

fn assert_one_glyph_per_char(galley: &Galley) {
    let glyphs: usize = galley.rows.iter().map(|row| row.row.glyphs.len()).sum();
    assert_eq!(
        glyphs,
        galley.job.text.chars().count(),
        "{:?}",
        galley.job.text
    );
}

#[test]
fn latin_words_and_numbers_keep_reading_order_inside_arabic() {
    let ctx = context();
    // (text, visual row from left to right)
    let cases = [
        (
            "يمكن للمسؤول تشغيله باستخدام powercfg /hibernate on.",
            format!(
                ".powercfg /hibernate on {} {} {} {}",
                reversed("باستخدام"),
                reversed("تشغيله"),
                reversed("للمسؤول"),
                reversed("يمكن")
            ),
        ),
        (
            "عند اكتمال جميع التنزيلات في Main queue",
            format!("Main queue {}", reversed("عند اكتمال جميع التنزيلات في")),
        ),
        (
            "نسخة من Download Engine تبدأ مع Windows",
            format!(
                "Windows {} Download Engine {}",
                reversed("تبدأ مع"),
                reversed("نسخة من")
            ),
        ),
        (
            "إيقاف التشغيل خلال 60 ثانية",
            format!("{} 60 {}", reversed("ثانية"), reversed("إيقاف التشغيل خلال")),
        ),
        // Digits after Arabic letters are Arabic numbers (UAX #9 rule W2), so the
        // percent sign follows them to the left, the usual Arabic presentation. A
        // number and a Latin unit are separate runs. Right to left this reads
        // "62.4", "%", "من", "8.4", "GB".
        (
            "تم تنزيل 62.4% من 8.4 GB",
            format!("GB 8.4 {} %62.4 {}", reversed("من"), reversed("تم تنزيل")),
        ),
        // After Latin text, the percent sign stays with the number.
        (
            "اكتمل Windows 62.4%",
            format!("Windows 62.4% {}", reversed("اكتمل")),
        ),
        // Arabic-Indic digits are written left to right as well.
        (
            "بعد ٦٠ ثانية",
            format!("{} ٦٠ {}", reversed("ثانية"), reversed("بعد")),
        ),
        // Brackets follow the text they enclose (UAX #9 rule N0).
        (
            "قائمة Download Engine (.json)",
            format!("Download Engine (.json) {}", reversed("قائمة")),
        ),
        // Brackets around Latin text inside Arabic stay in the Arabic run, where the
        // shaper mirrors them; the stored characters are the logical ones.
        (
            "إضافة تنزيل (Ctrl+N)",
            format!(")Ctrl+N( {}", reversed("إضافة تنزيل")),
        ),
    ];
    for (text, expected) in cases {
        let galley = layout(&ctx, text, f32::INFINITY);
        assert_eq!(visual_rows(&galley), [expected], "{text:?}");
        assert_one_glyph_per_char(&galley);
    }
}

#[test]
fn arabic_text_starting_with_latin_or_a_number_reads_right_to_left() {
    let ctx = context();
    let cases = [
        (
            "SHA-256 المتوقع (اختياري)",
            format!("{} {} SHA-256", reversed("(اختياري)"), reversed("المتوقع")),
        ),
        (
            "793.2 MB من 8.4 GB",
            format!("GB 8.4 {} MB 793.2", reversed("من")),
        ),
    ];
    for (text, expected) in cases {
        let rows = visual_rows(&layout(&ctx, text, f32::INFINITY));
        assert_eq!(rows, [expected], "{text:?}");
    }
}

#[test]
fn arabic_only_text_is_one_right_to_left_run() {
    let ctx = context();
    for text in ["تنزيل جديد", "قائمة الانتظار الرئيسية", "تنزيلات مكتملة"]
    {
        let galley = layout(&ctx, text, f32::INFINITY);
        assert_eq!(visual_rows(&galley), [reversed(text)], "{text:?}");
        assert_one_glyph_per_char(&galley);
    }
}

/// Glyph positions of each word relative to the word's left edge, as drawn.
fn word_offsets(galley: &Galley) -> Vec<(String, Vec<f32>)> {
    let mut words = Vec::new();
    for row in &galley.rows {
        let mut word: Option<(String, Vec<f32>, f32)> = None;
        for glyph in &row.row.glyphs {
            if glyph.chr == ' ' {
                words.extend(word.take().map(|(text, offsets, _)| (text, offsets)));
                continue;
            }
            let (text, offsets, left) =
                word.get_or_insert_with(|| (String::new(), Vec::new(), glyph.pos.x));
            text.push(glyph.chr);
            offsets.push(glyph.pos.x - *left);
        }
        words.extend(word.map(|(text, offsets, _)| (text, offsets)));
    }
    words
}

#[test]
fn wrapped_arabic_rows_read_top_to_bottom() {
    let ctx = context();
    let text = "الإسبات متوقف أو غير مدعوم على هذا الكمبيوتر. يمكن للمسؤول تشغيله باستخدام powercfg /hibernate on.";
    let galley = layout(&ctx, text, 160.0);
    let rows = visual_rows(&galley);
    assert!(rows.len() >= 3, "{rows:?}");
    // The first row holds the start of the sentence, at its right end.
    assert!(
        rows[0].trim_end().ends_with(&reversed("الإسبات")),
        "{rows:?}"
    );
    // The last row ends the sentence, with the command in reading order.
    let last = rows.last().unwrap();
    assert!(last.starts_with(".powercfg /hibernate on"), "{rows:?}");
    // Read right to left and top to bottom, the Arabic words are in order.
    let arabic: Vec<String> = rows
        .iter()
        .flat_map(|row| {
            row.split(' ')
                .rev()
                .filter(|word| !word.is_empty() && !word.chars().any(|c| c.is_ascii_alphabetic()))
                .map(reversed)
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        arabic.join(" "),
        "الإسبات متوقف أو غير مدعوم على هذا الكمبيوتر. يمكن للمسؤول تشغيله باستخدام"
    );
    assert_one_glyph_per_char(&galley);

    // Moving clusters onto rows keeps the joined glyphs of every Arabic word exactly
    // as shaped. (Separate runs, such as "." and "powercfg", may differ by a pixel.)
    let one_row = word_offsets(&layout(&ctx, text, f32::INFINITY));
    let wrapped = word_offsets(&galley);
    assert_eq!(wrapped.len(), one_row.len());
    let mut compared = 0;
    for (word, offsets) in &wrapped {
        if word.chars().any(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let shaped = one_row.iter().find(|(other, _)| other == word).unwrap();
        assert_eq!(offsets, &shaped.1, "{word:?}");
        compared += 1;
    }
    assert_eq!(compared, 12);
}

#[test]
fn elided_arabic_keeps_the_start_of_the_text() {
    let ctx = context();
    let mut job = LayoutJob::simple(
        "قائمة الانتظار الرئيسية متوقفة مؤقتًا الآن".to_owned(),
        egui::FontId::proportional(13.0),
        Color32::WHITE,
        90.0,
    );
    job.wrap.max_rows = 1;
    let galley = galley(&ctx, job);
    assert!(galley.elided);
    let rows = visual_rows(&galley);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].starts_with('…'), "{rows:?}");
    assert!(rows[0].ends_with(&reversed("قائمة")), "{rows:?}");
}

#[test]
fn sections_of_a_right_to_left_paragraph_flow_right_to_left() {
    let ctx = context();
    let mut job = LayoutJob::default();
    let format = |color| TextFormat {
        font_id: egui::FontId::proportional(13.0),
        color,
        ..Default::default()
    };
    job.append("عند اكتمال ", 0.0, format(Color32::WHITE));
    job.append("Main queue", 0.0, format(Color32::YELLOW));
    let galley = galley(&ctx, job);
    assert_eq!(
        visual_rows(&galley),
        [format!("Main queue {}", reversed("عند اكتمال"))]
    );
}

#[test]
fn left_to_right_text_is_laid_out_in_logical_order() {
    let ctx = context();
    for text in [
        "Main queue · 62.4% of 8.4 GB",
        "Blender-4.5-windows-x64.msi",
    ] {
        let galley = layout(&ctx, text, f32::INFINITY);
        assert_eq!(visual_rows(&galley), [text]);
    }
}

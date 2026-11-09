//! Present the host's recent downloads in a native scrolling view.
use enkel_module_sdk::{
    Download, DownloadStatus, Tool,
    appkit::{Host, Module, ui},
    event,
};
use objc2::rc::Retained;
use objc2_app_kit::{NSButton, NSTextView, NSView};
use std::{fmt::Write, rc::Rc};

struct Downloads(Retained<NSTextView>);

impl Module for Downloads {
    // The browser owns transfers and sends a snapshot after mounting this view.
    fn mount(_: Host, parent: &NSView, _: &NSButton) -> Rc<Self> {
        let (scroll, text) = ui::text_editor("Downloads");
        ui::pin_edges(&scroll, parent);
        text.setString(&ui::text("No downloads yet"));
        Rc::new(Self(text))
    }

    // Draw only the bounded snapshots from the host's download owner.
    fn event(&self, kind: u32, _: u64, bytes: &[u8]) -> bool {
        if kind != event::DOWNLOADS {
            return false;
        }
        let Ok(downloads) = serde_json::from_slice::<Vec<Download>>(bytes) else {
            return false;
        };
        let mut display = String::new();
        for download in downloads {
            let name = download.name;
            match download.status {
                DownloadStatus::Waiting => writeln!(display, "{name}    Waiting to save"),
                DownloadStatus::Progress(percent) if percent >= 0 => {
                    writeln!(display, "{name}    {percent}%")
                }
                DownloadStatus::Progress(_) => writeln!(display, "{name}    Downloading…"),
                DownloadStatus::Complete => writeln!(display, "{name}    Complete"),
                DownloadStatus::Cancelled => writeln!(display, "{name}    Cancelled"),
                DownloadStatus::Failed => writeln!(display, "{name}    Failed"),
            }
            .expect("Writing into a string");
        }
        let value = if display.is_empty() {
            "No downloads yet"
        } else {
            &display
        };
        self.0.setString(&ui::text(value));
        true
    }
}

enkel_module_sdk::export_module!(Tool::Downloads, Downloads);

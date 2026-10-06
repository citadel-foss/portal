//! Portal's About window. On macOS it is AppKit's About panel, opened here rather than through
//! Tauri's predefined About item: that one hands AppKit the credits as a bare string, which it
//! sets flush against both edges of the panel's text view, and a bare string has nowhere to carry
//! an inset. On Linux the predefined item's GTK dialog takes its fields as they are.

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2::{AnyThread, MainThreadMarker};
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSAboutPanelOptionApplicationIcon, NSAboutPanelOptionApplicationName,
    NSAboutPanelOptionApplicationVersion, NSAboutPanelOptionCredits, NSApplication, NSColor,
    NSForegroundColorAttributeName, NSImage, NSLinkAttributeName, NSMutableParagraphStyle,
    NSParagraphStyleAttributeName, NSTextAlignment,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{
    ns_string, NSAttributedString, NSData, NSDictionary, NSMutableAttributedString, NSString, NSURL,
};

const REPO: &str = "github.com/citadel-foss/portal";
const LICENSE: &str = "Open source under the MIT License";

fn version() -> &'static str {
    include_str!("../../version.txt").trim()
}

#[cfg(target_os = "linux")]
pub fn metadata(app: &tauri::App) -> tauri::menu::AboutMetadata<'_> {
    tauri::menu::AboutMetadata {
        name: Some("Portal".into()),
        version: Some(version().into()),
        comments: Some(format!(
            "Build date {}\nCommit {}",
            env!("PORTAL_BUILD_DATE"),
            env!("PORTAL_COMMIT")
        )),
        copyright: Some(LICENSE.into()),
        website: Some(format!("https://{REPO}")),
        website_label: Some(REPO.into()),
        icon: app.default_window_icon().cloned(),
        ..Default::default()
    }
}

#[cfg(target_os = "macos")]
/// Points in from each side of the credits' text view.
const CREDITS_INSET: f64 = 14.0;

#[cfg(target_os = "macos")]
// Embedded rather than read from the bundle: a dev build has no bundle, and AppKit then shows a
// generic folder.
const ICON: &[u8] = include_bytes!("../icons/128x128@2x.png");

#[cfg(target_os = "macos")]
pub fn show() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("About panel requested off the main thread");
        return;
    };

    let style = NSMutableParagraphStyle::new();
    style.setAlignment(NSTextAlignment::Center);
    style.setFirstLineHeadIndent(CREDITS_INSET);
    style.setHeadIndent(CREDITS_INSET);
    // Negative: measured in from the trailing edge, not out from the leading one.
    style.setTailIndent(-CREDITS_INSET);
    let style: Retained<AnyObject> =
        Retained::into_super(Retained::into_super(Retained::into_super(style)));

    let credits = NSMutableAttributedString::new();
    // System label colours, so the text follows light and dark mode like the rest of the panel.
    let append = |text: &str, color: Retained<NSColor>, link: Option<Retained<NSURL>>| {
        let mut keys = unsafe {
            vec![
                NSParagraphStyleAttributeName,
                NSForegroundColorAttributeName,
            ]
        };
        let mut values = vec![
            style.clone(),
            Retained::into_super(Retained::into_super(color)),
        ];
        if let Some(link) = link {
            keys.push(unsafe { NSLinkAttributeName });
            values.push(Retained::into_super(Retained::into_super(link)));
        }
        let attributes: Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_retained_objects(&keys, &values);
        let run = unsafe {
            NSAttributedString::new_with_attributes(&NSString::from_str(text), &attributes)
        };
        credits.appendAttributedString(&run);
    };
    // The blank first and last lines are the top and bottom inset: paragraph spacing, the styled
    // way, is not reliably applied before a text view's first line or after its last. At the
    // credits' font size a line is about `CREDITS_INSET` tall, so all four sides come out even.
    append("\n", NSColor::labelColor(), None);
    for (label, value) in [
        ("Build date", env!("PORTAL_BUILD_DATE")),
        ("Commit", env!("PORTAL_COMMIT")),
    ] {
        append(&format!("{label}\n"), NSColor::secondaryLabelColor(), None);
        append(&format!("{value}\n\n"), NSColor::labelColor(), None);
    }
    append("Source\n", NSColor::secondaryLabelColor(), None);
    append(
        REPO,
        NSColor::linkColor(),
        NSURL::URLWithString(&NSString::from_str(&format!("https://{REPO}"))),
    );
    append("\n", NSColor::labelColor(), None);

    let mut keys: Vec<&NSString> = unsafe {
        vec![
            NSAboutPanelOptionApplicationName,
            NSAboutPanelOptionApplicationVersion,
            NSAboutPanelOptionCredits,
        ]
    };
    let mut values: Vec<Retained<AnyObject>> = vec![
        Retained::into_super(Retained::into_super(NSString::from_str("Portal"))),
        Retained::into_super(Retained::into_super(NSString::from_str(version()))),
        Retained::into_super(Retained::into_super(Retained::into_super(credits))),
    ];
    keys.push(ns_string!("Copyright"));
    values.push(Retained::into_super(Retained::into_super(
        NSString::from_str(LICENSE),
    )));
    if let Some(icon) = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(ICON)) {
        keys.push(unsafe { NSAboutPanelOptionApplicationIcon });
        values.push(Retained::into_super(Retained::into_super(icon)));
    }

    let options = NSDictionary::from_retained_objects(&keys, &values);
    unsafe {
        NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanelWithOptions(&options)
    };
}

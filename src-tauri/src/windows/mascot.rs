use tauri::{AppHandle, Manager, PhysicalPosition};

/// Gap between the mascot and the work-area edge, in physical pixels.
const MARGIN: i32 = 24;

/// Places the mascot in the bottom-right corner of the primary monitor's work area
/// (above the taskbar), then shows it. Remembering the position comes with settings (Task 7).
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(super::MASCOT) else {
        return Ok(());
    };

    if let Some(monitor) = window.primary_monitor()? {
        let area = monitor.work_area();
        let size = window.outer_size()?;
        let x = area.position.x + area.size.width as i32 - size.width as i32 - MARGIN;
        let y = area.position.y + area.size.height as i32 - size.height as i32 - MARGIN;
        window.set_position(PhysicalPosition::new(x, y))?;
    }

    window.show()
}

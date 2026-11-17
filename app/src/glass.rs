//! The window's glass: the frosted material (public API, Settings → Frost style)
//! or a blur radius (Settings → Blur, or `palette glass dispatch`).
//!
//! macOS has no public blur radius. Ghostty, kitty and WezTerm all use the same private
//! WindowServer call, `CGSSetWindowBackgroundBlurRadius`, and so does this. It is looked up by
//! name at run time (`dlsym`), never linked, so if a macOS update removes it Dispatch still
//! starts: the lookup fails, the material comes back, and Settings says why. No new crate:
//! the Objective-C calls needed (`windowNumber`, `setOpaque:`) go through `objc_msgSend`.
//!
//! **Solid** (transparency 0, or "Solid when not in front") is done by the window itself, not
//! only by the page: its background becomes the theme's opaque background colour and it is
//! marked opaque, so WindowServer stops blending it with the desktop (the page alone
//! left the window clear when it hadn't repainted, and kept paying for the blending).

use std::ffi::{CStr, c_void};

use dispatch::settings::{Frost, Look};
use tauri::{AppHandle, Manager};

/// Applies `look` on the main thread (AppKit insists). The result (`Err` = the private blur
/// call is missing or refused) arrives through `done`.
/// `solid` is the theme's background colour, used when the look is solid.
pub fn apply(
    app: &AppHandle,
    look: Look,
    solid: (u8, u8, u8),
    done: impl FnOnce(Result<(), String>) + Send + 'static,
) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || done(apply_now(&handle, look, solid)));
}

fn apply_now(app: &AppHandle, look: Look, solid: (u8, u8, u8)) -> Result<(), String> {
    let Some(w) = app.get_webview_window("main") else {
        return Ok(());
    };
    set_solid(&w, (look.transparency == 0).then_some(solid));
    let wants_radius = look.blur > 0 && look.transparency > 0;
    let radius = if wants_radius {
        // 0–100 like Ghostty's background-blur, which is the radius in points.
        set_radius(&w, look.blur as i32)
    } else {
        // Back to none, so a radius set earlier doesn't linger under the material.
        let _ = set_radius(&w, 0);
        Ok(())
    };
    let material = if look.transparency == 0 || (wants_radius && radius.is_ok()) {
        // Solid: the page covers the window, so a live material behind it would only cost
        // GPU time. Radius: it replaces the material.
        None
    } else {
        look.material
    };
    set_material(&w, material);
    radius
}

fn set_material(w: &tauri::WebviewWindow, material: Option<Frost>) {
    use tauri::window::{Effect, EffectState, EffectsBuilder};
    let Some(frost) = material else {
        let _ = w.set_effects(None::<tauri::utils::config::WindowEffectsConfig>);
        return;
    };
    let effect = match frost {
        Frost::Window => Effect::UnderWindowBackground,
        Frost::Sidebar => Effect::Sidebar,
        Frost::Popover => Effect::Popover,
        Frost::Hud => Effect::HudWindow,
    };
    // Always "active", so the frost stays when another app has focus.
    let _ = w.set_effects(
        EffectsBuilder::new()
            .effect(effect)
            .state(EffectState::Active)
            .build(),
    );
}

/// `Some(colour)`: an opaque window in that colour. `None`: see-through again.
fn set_solid(w: &tauri::WebviewWindow, colour: Option<(u8, u8, u8)>) {
    let window = w.as_ref().window();
    let _ = window.set_background_color(colour.map(|(r, g, b)| tauri::window::Color(r, g, b, 255)));
    #[cfg(target_os = "macos")]
    if let Ok(ns_window) = w.ns_window() {
        type SendBool = unsafe extern "C" fn(*mut c_void, *const c_void, bool);
        unsafe extern "C" {
            fn sel_registerName(name: *const std::ffi::c_char) -> *const c_void;
            fn objc_msgSend();
        }
        // SAFETY: `setOpaque:` is an NSWindow method taking one BOOL; main thread.
        unsafe {
            let send: SendBool = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            send(
                ns_window,
                sel_registerName(c"setOpaque:".as_ptr()),
                colour.is_some(),
            );
        }
    }
}

#[cfg(target_os = "macos")]
fn set_radius(w: &tauri::WebviewWindow, radius: i32) -> Result<(), String> {
    type MainConnection = unsafe extern "C" fn() -> i32;
    type SetRadius = unsafe extern "C" fn(i32, isize, i32) -> i32;
    type MsgSend = unsafe extern "C" fn(*mut c_void, *const c_void) -> isize;
    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
        fn sel_registerName(name: *const std::ffi::c_char) -> *const c_void;
        fn objc_msgSend();
    }
    const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
    let find = |name: &CStr| {
        // SAFETY: dlsym with RTLD_DEFAULT only looks a name up; null means "not there".
        let p = unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) };
        (!p.is_null()).then_some(p)
    };
    let missing = "this macOS no longer has the private blur call; using the Frost material";
    let conn = find(c"CGSMainConnectionID").ok_or(missing)?;
    let set = find(c"CGSSetWindowBackgroundBlurRadius").ok_or(missing)?;
    let ns_window = w.ns_window().map_err(|e| e.to_string())?;
    // SAFETY: the pointers come from dlsym for these exact C signatures (as Ghostty declares
    // them); `windowNumber` is an NSWindow method returning NSInteger; we're on the main thread.
    let code = unsafe {
        let conn: MainConnection = std::mem::transmute(conn);
        let set: SetRadius = std::mem::transmute(set);
        let send: MsgSend = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let number = send(ns_window, sel_registerName(c"windowNumber".as_ptr()));
        set(conn(), number, radius)
    };
    if code == 0 {
        Ok(())
    } else {
        Err(format!(
            "macOS refused the blur (error {code}); using the Frost material"
        ))
    }
}

#[cfg(not(target_os = "macos"))]
fn set_radius(_w: &tauri::WebviewWindow, _radius: i32) -> Result<(), String> {
    Err("blur is macOS only".into())
}

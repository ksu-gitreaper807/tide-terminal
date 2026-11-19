//! Apply a desktop-supplied xdg-activation token to the existing Wayland surface.
//! egui/winit's normal Focus command is deliberately a no-op on Wayland.
use anyhow::{Context, Result};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::{
    Connection, Dispatch, Proxy,
    protocol::{wl_registry, wl_surface},
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1;

#[derive(Default)]
struct Registry {
    activation: Option<xdg_activation_v1::XdgActivationV1>,
}
impl Dispatch<wl_registry::WlRegistry, ()> for Registry {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &wayland_client::QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
            && interface == "xdg_activation_v1"
        {
            state.activation = Some(registry.bind(name, 1, qh, ()));
        }
    }
}
wayland_client::delegate_noop!(Registry: ignore xdg_activation_v1::XdgActivationV1);

pub fn activate(frame: &eframe::Frame, token: Option<&str>) -> Result<()> {
    match frame.window_handle()?.as_raw() {
        RawWindowHandle::Xlib(window) => {
            return activate_x11(window.window as u32, token.unwrap_or_default());
        }
        RawWindowHandle::Xcb(window) => {
            return activate_x11(window.window.get(), token.unwrap_or_default());
        }
        _ => {}
    }
    let Some(token) = token else {
        return Ok(());
    };
    let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(window)) = (
        frame.display_handle()?.as_raw(),
        frame.window_handle()?.as_raw(),
    ) else {
        return Ok(());
    };
    // SAFETY: eframe owns these handles. They are borrowed only during this call
    // on the GUI thread; no proxy/connection escapes the frame's lifetime. The
    // foreign backend does not disconnect or destroy eframe's display/surface.
    let backend = unsafe {
        wayland_backend::client::Backend::from_foreign_display(display.display.as_ptr().cast())
    };
    let connection = Connection::from_backend(backend);
    let surface_id = unsafe {
        wayland_backend::client::ObjectId::from_ptr(
            wl_surface::WlSurface::interface(),
            window.surface.as_ptr().cast(),
        )
    }?;
    let surface = wl_surface::WlSurface::from_id(&connection, surface_id)?;
    let mut queue = connection.new_event_queue::<Registry>();
    let _registry = connection.display().get_registry(&queue.handle(), ());
    let mut state = Registry::default();
    queue.roundtrip(&mut state)?;
    let activation = state
        .activation
        .context("Desktop does not support window activation")?;
    activation.activate(token.to_owned(), &surface);
    activation.destroy();
    connection.flush()?;
    Ok(())
}

fn activate_x11(window: u32, token: &str) -> Result<()> {
    use x11rb::{
        connection::Connection as _,
        protocol::xproto::{
            AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, InputFocus, PropMode,
        },
        wrapper::ConnectionExt as _,
    };
    let (connection, screen) = x11rb::connect(None)?;
    let atom =
        |name: &[u8]| -> Result<u32> { Ok(connection.intern_atom(false, name)?.reply()?.atom) };
    let timestamp = token
        .rsplit_once("_TIME")
        .and_then(|(_, time)| time.parse::<u32>().ok())
        .unwrap_or(0);
    if !token.is_empty() {
        connection.change_property8(
            PropMode::REPLACE,
            window,
            atom(b"_NET_STARTUP_ID")?,
            atom(b"UTF8_STRING")?,
            token.as_bytes(),
        )?;
    }
    if timestamp != 0 {
        connection.change_property32(
            PropMode::REPLACE,
            window,
            atom(b"_NET_WM_USER_TIME")?,
            AtomEnum::CARDINAL,
            &[timestamp],
        )?;
    }
    connection.map_window(window)?;
    let root = connection.setup().roots[screen].root;
    let manager = connection
        .get_property(
            false,
            root,
            atom(b"_NET_SUPPORTING_WM_CHECK")?,
            AtomEnum::WINDOW,
            0,
            1,
        )?
        .reply()?;
    if manager.value_len == 0 {
        // Bare X servers have no window manager to handle EWMH requests.
        connection.set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)?;
    }
    // This is an explicit user click on a desktop window-switching action, not
    // an application's unsolicited request for focus (source indication 1).
    let event = ClientMessageEvent::new(
        32,
        window,
        atom(b"_NET_ACTIVE_WINDOW")?,
        [2, timestamp, 0, 0, 0],
    );
    connection
        .send_event(
            false,
            connection.setup().roots[screen].root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )?
        .check()?;
    connection.flush()?;
    Ok(())
}

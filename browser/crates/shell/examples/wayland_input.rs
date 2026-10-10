//! Explicit Wayland GUI-test pointer. Not linked into the browser binary.
use wayland_client::{
    delegate_noop,
    protocol::{wl_pointer, wl_registry},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1 as manager, zwlr_virtual_pointer_v1 as pointer,
};
#[derive(Default)]
struct State {
    manager: Option<manager::ZwlrVirtualPointerManagerV1>,
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "zwlr_virtual_pointer_manager_v1" {
                state.manager = Some(registry.bind(name, version.min(2), qh, ()));
            }
        }
    }
}
delegate_noop!(State: ignore manager::ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore pointer::ZwlrVirtualPointerV1);
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 4 {
        return Err(
            "usage: wayland_input X Y OUTPUT_WIDTH OUTPUT_HEIGHT [click|scroll DELTA]".into(),
        );
    }
    let x: u32 = args[0].parse()?;
    let y: u32 = args[1].parse()?;
    let width: u32 = args[2].parse()?;
    let height: u32 = args[3].parse()?;
    let connection = Connection::connect_to_env()?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    let mut state = State::default();
    connection.display().get_registry(&qh, ());
    queue.roundtrip(&mut state)?;
    let manager = state
        .manager
        .ok_or("compositor does not expose virtual pointers")?;
    let pointer = manager.create_virtual_pointer(None, &qh, ());
    pointer.motion_absolute(1, x, y, width, height);
    pointer.frame();
    match args.get(4).map(String::as_str) {
        Some("click") => {
            pointer.button(2, 272, wl_pointer::ButtonState::Pressed);
            pointer.frame();
            pointer.button(3, 272, wl_pointer::ButtonState::Released);
            pointer.frame();
        }
        Some("scroll") => {
            let delta: f64 = args.get(5).ok_or("scroll delta missing")?.parse()?;
            pointer.axis_source(wl_pointer::AxisSource::Wheel);
            pointer.axis_discrete(
                2,
                wl_pointer::Axis::VerticalScroll,
                delta,
                (delta / 15.0) as i32,
            );
            pointer.frame();
        }
        None => {}
        _ => return Err("unknown pointer action".into()),
    }
    connection.flush()?;
    queue.roundtrip(&mut State::default())?;
    pointer.destroy();
    manager.destroy();
    connection.flush()?;
    Ok(())
}

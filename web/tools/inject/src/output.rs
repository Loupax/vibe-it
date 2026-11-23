// C6b: `inject output W H SCALE` sets the nested compositor's (only) output to W x H pixels at
// SCALE through wlr-output-management (what wlr-randr does), so a live test can change the
// scale while the shell runs (a window moved to another monitor).
use wayland_client::{protocol::wl_registry, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::ZwlrOutputModeV1,
};

#[derive(Default)]
struct Om {
    mgr: Option<ZwlrOutputManagerV1>,
    heads: Vec<ZwlrOutputHeadV1>,
    serial: Option<u32>,
    result: Option<&'static str>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Om {
    fn event(
        s: &mut Self,
        r: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = e
        {
            if interface == "zwlr_output_manager_v1" {
                s.mgr = Some(r.bind(name, version.min(4), qh, ()));
            }
        }
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for Om {
    fn event(
        s: &mut Self,
        _: &ZwlrOutputManagerV1,
        e: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            zwlr_output_manager_v1::Event::Head { head } => s.heads.push(head),
            zwlr_output_manager_v1::Event::Done { serial } => s.serial = Some(serial),
            _ => {}
        }
    }
    wayland_client::event_created_child!(Om, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for Om {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputHeadV1,
        _: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
    wayland_client::event_created_child!(Om, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for Om {
    fn event(
        s: &mut Self,
        _: &ZwlrOutputConfigurationV1,
        e: zwlr_output_configuration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        s.result = Some(match e {
            zwlr_output_configuration_v1::Event::Succeeded => "succeeded",
            zwlr_output_configuration_v1::Event::Failed => "failed",
            zwlr_output_configuration_v1::Event::Cancelled => "cancelled",
            _ => "unknown",
        });
    }
}

macro_rules! noop { ($($t:ty),*) => { $(impl Dispatch<$t, ()> for Om { fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {} })* } }
noop!(ZwlrOutputModeV1, ZwlrOutputConfigurationHeadV1);

pub fn set(w: i32, h: i32, scale: f64) {
    let conn = Connection::connect_to_env().unwrap();
    let mut q = conn.new_event_queue();
    let qh = q.handle();
    conn.display().get_registry(&qh, ());
    let mut st = Om::default();
    q.roundtrip(&mut st).unwrap();
    q.roundtrip(&mut st).unwrap();
    let mgr = st.mgr.clone().expect("no zwlr_output_manager_v1");
    let serial = st.serial.expect("no output manager done event");
    let head = st.heads.first().expect("no output head").clone();
    let cfg = mgr.create_configuration(serial, &qh, ());
    let ch = cfg.enable_head(&head, &qh, ());
    ch.set_custom_mode(w, h, 0);
    ch.set_scale(scale);
    cfg.apply();
    while st.result.is_none() {
        q.blocking_dispatch(&mut st).unwrap();
    }
    let r = st.result.unwrap();
    println!("output {w}x{h} scale {scale}: {r}");
    if r != "succeeded" {
        std::process::exit(1);
    }
}

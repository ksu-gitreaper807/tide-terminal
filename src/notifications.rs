//! Desktop alerts have one bus connection per GUI, never block the UI or PTY,
//! and carry only a prompt description (never the user's response).
use crossbeam_channel::{Receiver, bounded};
use futures_lite::{StreamExt, future};
use std::collections::HashMap;
use zbus::zvariant::Value;

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

#[derive(Debug)]
pub struct Activation {
    pub session: u64,
    pub run: u64,
    pub token: Option<String>,
}
enum Request {
    Show {
        session: u64,
        run: u64,
        body: String,
    },
    Close(u64),
}
struct Pending {
    session: u64,
    run: u64,
    token: Option<String>,
}

pub struct Notifications {
    requests: async_channel::Sender<Request>,
    pub activations: Receiver<Activation>,
}
impl Notifications {
    pub fn new(wake: impl Fn() + Send + 'static) -> Self {
        Self::connect(None, wake)
    }
    fn connect(address: Option<String>, wake: impl Fn() + Send + 'static) -> Self {
        let (requests, receiver) = async_channel::bounded(128);
        let (sender, activations) = bounded(128);
        std::thread::spawn(move || {
            let activate = move |activation| {
                if sender.try_send(activation).is_ok() {
                    wake();
                }
            };
            // Lazy connection, including ignoring early prompt-clear events.
            // A missing/restarted desktop service must not disable later alerts.
            while let Ok(first) = receiver.recv_blocking() {
                if matches!(first, Request::Show { .. }) {
                    let _ =
                        future::block_on(serve(first, &receiver, address.as_deref(), &activate));
                }
            }
        });
        Self {
            requests,
            activations,
        }
    }
    pub fn show(&self, session: u64, run: u64, body: String) {
        let _ = self.requests.try_send(Request::Show { session, run, body });
    }
    pub fn close(&self, session: u64) {
        let _ = self.requests.try_send(Request::Close(session));
    }
}
impl Drop for Notifications {
    fn drop(&mut self) {
        self.requests.close();
    }
}

fn markup_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn icon_data() -> Option<Value<'static>> {
    #[cfg(feature = "desktop")]
    {
        let image = image::load_from_memory_with_format(
            include_bytes!("../assets/branding/window-icon.png"),
            image::ImageFormat::Png,
        )
        .ok()?
        .into_rgba8();
        let (width, height) = image.dimensions();
        Some(Value::from((
            width as i32,
            height as i32,
            width as i32 * 4,
            true,
            8i32,
            4i32,
            image.into_raw(),
        )))
    }
    #[cfg(not(feature = "desktop"))]
    {
        None
    }
}

async fn close(proxy: &zbus::Proxy<'_>, id: u32) {
    let _: zbus::Result<()> = proxy.call("CloseNotification", &(id,)).await;
}
async fn serve(
    first: Request,
    requests: &async_channel::Receiver<Request>,
    address: Option<&str>,
    activate: impl Fn(Activation),
) -> zbus::Result<()> {
    let builder = if let Some(address) = address {
        zbus::connection::Builder::address(address)?
    } else {
        zbus::connection::Builder::session()?
    };
    let connection = builder
        .method_timeout(std::time::Duration::from_secs(3))
        .build()
        .await?;
    let proxy = zbus::Proxy::new(&connection, SERVICE, PATH, SERVICE).await?;
    // Subscribe before Notify: an immediate click/close must not be lost.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(SERVICE)?
        .path(PATH)?
        .interface(SERVICE)?
        .build();
    let mut signals = zbus::MessageStream::for_match_rule(rule, &connection, Some(128)).await?;
    let mut pending: HashMap<u32, Pending> = HashMap::new();
    let mut next = Some(first);
    loop {
        enum Incoming {
            Request(Result<Request, async_channel::RecvError>),
            Signal(Option<zbus::Result<zbus::Message>>),
        }
        let incoming = if let Some(request) = next.take() {
            Incoming::Request(Ok(request))
        } else {
            future::or(async { Incoming::Request(requests.recv().await) }, async {
                Incoming::Signal(signals.next().await)
            })
            .await
        };
        match incoming {
            Incoming::Request(Ok(Request::Show { session, run, body })) => {
                let previous = pending
                    .iter()
                    .find(|(_, p)| p.session == session)
                    .map(|(id, _)| *id)
                    .unwrap_or(0);
                if previous != 0 {
                    pending.remove(&previous);
                    close(&proxy, previous).await;
                }
                let mut hints = HashMap::from([("desktop-entry", Value::from("tide-terminal"))]);
                if let Some(image) = icon_data() {
                    hints.insert("image-data", image);
                }
                let id: u32 = proxy
                    .call(
                        "Notify",
                        &(
                            "Tide",
                            0u32,
                            "tide-terminal",
                            "Tide · input needed",
                            markup_text(&body),
                            vec!["default", "Open Tide"],
                            hints,
                            -1i32,
                        ),
                    )
                    .await?;
                pending.insert(
                    id,
                    Pending {
                        session,
                        run,
                        token: None,
                    },
                );
            }
            Incoming::Request(Ok(Request::Close(session))) => {
                if let Some(id) = pending
                    .iter()
                    .find(|(_, p)| p.session == session)
                    .map(|(id, _)| *id)
                {
                    pending.remove(&id);
                    close(&proxy, id).await;
                }
            }
            Incoming::Signal(Some(Ok(message))) => {
                match message.header().member().map(|name| name.as_str()) {
                    Some("ActivationToken") => {
                        if let Ok((id, token)) = message.body().deserialize::<(u32, String)>()
                            && let Some(p) = pending.get_mut(&id)
                        {
                            p.token = Some(token);
                        }
                    }
                    Some("ActionInvoked") => {
                        if let Ok((id, action)) = message.body().deserialize::<(u32, String)>()
                            && action == "default"
                            && let Some(p) = pending.remove(&id)
                        {
                            activate(Activation {
                                session: p.session,
                                run: p.run,
                                token: p.token,
                            });
                            close(&proxy, id).await;
                        }
                    }
                    Some("NotificationClosed") => {
                        if let Ok((id, _)) = message.body().deserialize::<(u32, u32)>() {
                            pending.remove(&id);
                        }
                    }
                    _ => {}
                }
            }
            _ => break,
        }
    }
    for id in pending.into_keys() {
        close(&proxy, id).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn prompt_descriptions_are_plain_text() {
        assert_eq!(
            super::markup_text("Password for <demo> & PIN:"),
            "Password for &lt;demo&gt; &amp; PIN:"
        );
    }
}

#[cfg(test)]
mod bus_tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        time::Duration,
    };
    use zbus::zvariant::OwnedValue;
    #[derive(Debug)]
    struct Notice {
        id: u32,
        app: String,
        icon: String,
        title: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
    }
    struct Server {
        next: u32,
        notices: crossbeam_channel::Sender<Notice>,
        closed: crossbeam_channel::Sender<u32>,
    }
    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Server {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &mut self,
            app: String,
            replaces: u32,
            icon: String,
            title: String,
            body: String,
            actions: Vec<String>,
            hints: HashMap<String, OwnedValue>,
            _timeout: i32,
        ) -> u32 {
            let id = if replaces == 0 {
                self.next += 1;
                self.next
            } else {
                replaces
            };
            self.notices
                .send(Notice {
                    id,
                    app,
                    icon,
                    title,
                    body,
                    actions,
                    hints,
                })
                .unwrap();
            id
        }
        fn close_notification(&self, id: u32) {
            let _ = self.closed.send(id);
        }
    }
    struct Bus(std::process::Child);
    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[test]
    fn desktop_alert_icon_actions_tokens_replacement_and_cleanup() {
        // Own private bus: no environment changes and no notifications on the user's desktop.
        let child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut bus = Bus(child);
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim();
        let (notices, received) = bounded(32);
        let (closed, closures) = bounded(32);
        let server = zbus::blocking::connection::Builder::address(address)
            .unwrap()
            .name(SERVICE)
            .unwrap()
            .serve_at(
                PATH,
                Server {
                    next: 0,
                    notices,
                    closed,
                },
            )
            .unwrap()
            .build()
            .unwrap();
        let signal = |member: &str, body: &(u32, &str)| {
            server
                .emit_signal(None::<&str>, PATH, SERVICE, member, body)
                .unwrap()
        };
        let client = Notifications::connect(Some(address.into()), || {});
        let timeout = Duration::from_secs(5);
        client.show(11, 101, "Password for <test> & PIN:".into());
        let notice = received.recv_timeout(timeout).unwrap();
        assert_eq!(notice.app, "Tide");
        assert_eq!(notice.icon, "tide-terminal");
        assert_eq!(notice.title, "Tide · input needed");
        assert_eq!(notice.body, "Password for &lt;test&gt; &amp; PIN:");
        assert_eq!(notice.actions, ["default", "Open Tide"]);
        assert_eq!(
            <&str>::try_from(&notice.hints["desktop-entry"]).unwrap(),
            "tide-terminal"
        );
        #[cfg(feature = "desktop")]
        {
            let image = notice.hints["image-data"].try_clone().unwrap();
            let (w, h, stride, alpha, bits, channels, rgba): (
                i32,
                i32,
                i32,
                bool,
                i32,
                i32,
                Vec<u8>,
            ) = image.try_into().unwrap();
            assert_eq!(
                (w, h, stride, alpha, bits, channels),
                (128, 128, 512, true, 8, 4)
            );
            let expected =
                image::load_from_memory(include_bytes!("../assets/branding/window-icon.png"))
                    .unwrap()
                    .into_rgba8();
            assert_eq!(rgba, expected.into_raw());
        }
        signal("ActionInvoked", &(999, "default"));
        signal("ActionInvoked", &(notice.id, "unknown"));
        signal("ActivationToken", &(999, "wrong-token"));
        signal("ActivationToken", &(notice.id, "desktop-click-token"));
        signal("ActionInvoked", &(notice.id, "default"));
        let clicked = client.activations.recv_timeout(timeout).unwrap();
        assert_eq!(
            (clicked.session, clicked.run, clicked.token.as_deref()),
            (11, 101, Some("desktop-click-token"))
        );
        assert_eq!(closures.recv_timeout(timeout).unwrap(), notice.id);
        // Another tab, then a new prompt in that tab: replace rather than accumulate.
        client.show(
            22,
            202,
            "Place your finger on the fingerprint reader".into(),
        );
        let first = received.recv_timeout(timeout).unwrap();
        signal("ActivationToken", &(first.id, "old-token"));
        client.show(22, 202, "[sudo] password for test:".into());
        let replaced = received.recv_timeout(timeout).unwrap();
        assert_ne!(first.id, replaced.id);
        assert_eq!(closures.recv_timeout(timeout).unwrap(), first.id);
        signal("ActionInvoked", &(replaced.id, "default"));
        let clicked = client.activations.recv_timeout(timeout).unwrap();
        assert_eq!(
            (clicked.session, clicked.run, clicked.token),
            (22, 202, None)
        );
        assert_eq!(closures.recv_timeout(timeout).unwrap(), replaced.id);
        // User already responded: stale clicks must never switch or raise a window.
        client.show(33, 303, "Continue? [y/N]".into());
        let stale = received.recv_timeout(timeout).unwrap();
        client.close(33);
        assert_eq!(closures.recv_timeout(timeout).unwrap(), stale.id);
        signal("ActionInvoked", &(stale.id, "default"));
        // Dismissal is not activation; dropping the GUI withdraws remaining alerts.
        client.show(44, 404, "Press Enter".into());
        let dismissed = received.recv_timeout(timeout).unwrap();
        server
            .emit_signal(
                None::<&str>,
                PATH,
                SERVICE,
                "NotificationClosed",
                &(dismissed.id, 2u32),
            )
            .unwrap();
        signal("ActionInvoked", &(dismissed.id, "default"));
        client.show(55, 505, "Password:".into());
        let final_notice = received.recv_timeout(timeout).unwrap();
        assert!(
            client
                .activations
                .recv_timeout(Duration::from_millis(150))
                .is_err()
        );
        drop(client);
        assert_eq!(closures.recv_timeout(timeout).unwrap(), final_notice.id);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use crate::{
        config::{Binding, Config},
        input::{KeyEvent, Learner, Matcher},
        platform, runner,
    };
    use anyhow::{bail, Context, Result};
    use evdev::{
        uinput::VirtualDevice, AbsInfo, Device, EventType, InputEvent, KeyCode, UinputAbsSetup,
    };
    use serde::{Deserialize, Serialize};
    use std::{
        collections::BTreeSet,
        fs,
        io::{BufRead, BufReader, Read, Write},
        os::{
            fd::AsRawFd,
            unix::{
                fs::PermissionsExt,
                net::{UnixListener, UnixStream},
            },
        },
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    const SOCKET: &str = "/run/coconut/input.sock";
    const LEARN_TIMEOUT: Duration = Duration::from_secs(60);
    #[derive(Serialize, Deserialize)]
    struct Request {
        session: String,
        mode: String,
        binding: Option<Binding>,
    }
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum Message {
        Hello,
        Status {
            version: String,
            keyboards: Vec<DeviceInfo>,
        },
        Trigger {
            #[serde(default)]
            sent_at_ms: Option<u64>,
        },
        Learned {
            binding: Binding,
            name: String,
        },
        Error {
            message: String,
        },
        Emergency,
    }
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct DeviceInfo {
        pub id: String,
        pub name: String,
        pub path: String,
        pub composite: bool,
        pub copilot_keys: Vec<String>,
    }
    fn send(s: &mut UnixStream, m: &Message) -> Result<()> {
        let mut v = serde_json::to_vec(m)?;
        v.push(b'\n');
        s.write_all(&v)?;
        Ok(())
    }
    fn unix_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
    fn session() -> Result<String> {
        std::env::var("XDG_SESSION_ID")
            .context("No XDG_SESSION_ID. Run coconut in a local graphical login session.")
    }
    fn allowed(uid: u32, session: &str) -> bool {
        use gio::prelude::*;
        if session.is_empty()
            || session.len() > 64
            || !session
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return false;
        }
        let query = (|| -> Option<bool> {
            let bus = gio::bus_get_sync(gio::BusType::System, None::<&gio::Cancellable>).ok()?;
            let result = bus
                .call_sync(
                    Some("org.freedesktop.login1"),
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                    "GetSession",
                    Some(&(session,).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    500,
                    None::<&gio::Cancellable>,
                )
                .ok()?;
            let (object,) = result.get::<(gio::glib::variant::ObjectPath,)>()?;
            let result = bus
                .call_sync(
                    Some("org.freedesktop.login1"),
                    object.as_str(),
                    "org.freedesktop.DBus.Properties",
                    "GetAll",
                    Some(&("org.freedesktop.login1.Session",).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    500,
                    None::<&gio::Cancellable>,
                )
                .ok()?;
            let (p,) = result.get::<(std::collections::HashMap<String, gio::glib::Variant>,)>()?;
            let (user, _) = p
                .get("User")?
                .get::<(u32, gio::glib::variant::ObjectPath)>()?;
            let (seat, _) = p
                .get("Seat")?
                .get::<(String, gio::glib::variant::ObjectPath)>()?;
            Some(
                user == uid
                    && seat == "seat0"
                    && p.get("Active")?.get::<bool>()?
                    && !p.get("LockedHint")?.get::<bool>()?
                    && !p.get("Remote")?.get::<bool>()?
                    && matches!(p.get("Type")?.str()?, "wayland" | "x11"),
            )
        })();
        query.unwrap_or(false)
    }
    fn peer_uid(s: &UnixStream) -> Result<u32> {
        let mut c: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                s.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut c as *mut libc::ucred).cast(),
                &mut len,
            )
        } != 0
        {
            bail!("Cannot identify peer")
        }
        Ok(c.uid)
    }
    pub fn connect(mode: &str, binding: Option<Binding>) -> Result<UnixStream> {
        let mut s = UnixStream::connect(SOCKET)
            .context("Input service unavailable. Run coconut setup to install it.")?;
        let r = Request {
            session: session()?,
            mode: mode.into(),
            binding,
        };
        let mut bytes = serde_json::to_vec(&r)?;
        bytes.push(b'\n');
        s.write_all(&bytes)?;
        Ok(s)
    }
    struct Keyboard {
        dev: Device,
        virtual_dev: VirtualDevice,
        path: String,
        id: String,
        name: String,
        matcher: Option<Matcher>,
        learner: Learner,
        grabbed: bool,
        down: BTreeSet<u16>,
    }
    fn virtual_keyboard(
        dev: &Device,
        keys: &evdev::AttributeSetRef<KeyCode>,
    ) -> Result<VirtualDevice> {
        let mut builder = VirtualDevice::builder()?
            .name("Coconut virtual keyboard")
            .input_id(dev.input_id())
            .with_keys(keys)?;
        if let Some(axes) = dev.supported_relative_axes() {
            builder = builder.with_relative_axes(axes)?;
        }
        if let Some(axes) = dev.supported_absolute_axes() {
            let state = dev.get_abs_state()?;
            for axis in axes.iter() {
                let info = state[usize::from(axis.0)];
                builder = builder.with_absolute_axis(&UinputAbsSetup::new(
                    axis,
                    AbsInfo::new(
                        info.value,
                        info.minimum,
                        info.maximum,
                        info.fuzz,
                        info.flat,
                        info.resolution,
                    ),
                ))?;
            }
        }
        if let Some(switches) = dev.supported_switches() {
            builder = builder.with_switches(switches)?;
        }
        if let Some(misc) = dev.misc_properties() {
            builder = builder.with_msc(misc)?;
        }
        builder = builder.with_properties(dev.properties())?;
        Ok(builder.build()?)
    }
    impl Drop for Keyboard {
        fn drop(&mut self) {
            let events: Vec<_> = self
                .down
                .iter()
                .map(|k| InputEvent::new(EventType::KEY.0, *k, 0))
                .collect();
            let _ = self.virtual_dev.emit(&events);
            if self.grabbed {
                let _ = self.dev.ungrab();
            }
        }
    }
    fn keyboards(exclude: &BTreeSet<String>, target: Option<&str>) -> Vec<Keyboard> {
        evdev::enumerate()
            .filter_map(|(path, dev)| {
                if exclude.contains(path.to_string_lossy().as_ref()) {
                    return None;
                }
                let name = dev.name().unwrap_or("Keyboard").to_string();
                if name.starts_with("Coconut ") {
                    return None;
                }
                let keys = dev.supported_keys()?;
                if !keys.contains(KeyCode::KEY_A)
                    && !keys.contains(KeyCode::KEY_F23)
                    && !keys.contains(KeyCode::KEY_ASSISTANT)
                {
                    return None;
                }
                let input = dev.input_id();
                let id = format!(
                    "{:04x}:{:04x}:{}:{}",
                    input.vendor(),
                    input.product(),
                    dev.physical_path().unwrap_or(""),
                    name
                );
                if target.is_some_and(|expected| expected != id) {
                    return None;
                }
                let virt = virtual_keyboard(&dev, keys).ok()?;
                unsafe {
                    let flags = libc::fcntl(dev.as_raw_fd(), libc::F_GETFL);
                    libc::fcntl(dev.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
                }
                Some(Keyboard {
                    dev,
                    virtual_dev: virt,
                    path: path.to_string_lossy().into(),
                    id,
                    name,
                    matcher: None,
                    learner: Learner::default(),
                    grabbed: false,
                    down: BTreeSet::new(),
                })
            })
            .collect()
    }
    fn emit(k: &mut Keyboard, events: Vec<KeyEvent>) -> Result<()> {
        let es: Vec<_> = events
            .iter()
            .map(|e| {
                if e.value == 1 {
                    k.down.insert(e.code);
                } else if e.value == 0 {
                    k.down.remove(&e.code);
                }
                InputEvent::new(EventType::KEY.0, e.code, e.value)
            })
            .collect();
        if !es.is_empty() {
            k.virtual_dev.emit(&es)?;
        }
        Ok(())
    }
    fn release(k: &mut Keyboard) {
        let es: Vec<_> = k
            .down
            .iter()
            .map(|code| KeyEvent {
                code: *code,
                value: 0,
            })
            .collect();
        let _ = emit(k, es);
        if k.grabbed {
            let _ = k.dev.ungrab();
        }
        k.grabbed = false;
        k.matcher = None;
    }
    struct Client {
        stream: UnixStream,
        uid: u32,
        request: Request,
        authorized: Arc<AtomicBool>,
        started: Instant,
    }
    pub fn run() -> Result<()> {
        if unsafe { libc::geteuid() } != 0 {
            bail!("Input service must run as root")
        }
        fs::create_dir_all("/run/coconut")?;
        if std::path::Path::new(SOCKET).exists() {
            fs::remove_file(SOCKET)?;
        }
        let listener = UnixListener::bind(SOCKET)?;
        fs::set_permissions(SOCKET, fs::Permissions::from_mode(0o666))?;
        listener.set_nonblocking(true)?;
        println!(
            "COCONUT input service {} ready at {SOCKET}",
            env!("CARGO_PKG_VERSION")
        );
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || loop {
            if let Ok((mut stream, _)) = listener.accept() {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
                    let Ok(uid) = peer_uid(&stream) else { return };
                    let mut raw = vec![];
                    let mut byte = [0];
                    while raw.len() < 8192 && stream.read(&mut byte).unwrap_or(0) == 1 {
                        if byte[0] == b'\n' {
                            break;
                        }
                        raw.push(byte[0]);
                    }
                    let Ok(r) = serde_json::from_slice::<Request>(&raw) else {
                        return;
                    };
                    let valid_binding = r.binding.as_ref().is_none_or(Binding::valid);
                    if !valid_binding
                        || !matches!(r.mode.as_str(), "agent" | "learn" | "probe")
                        || !allowed(uid, &r.session)
                    {
                        eprintln!(
                            "Rejected {} client from uid {} for session {}",
                            r.mode, uid, r.session
                        );
                        let _=send(&mut stream,&Message::Error{message:"Requires an active, unlocked seat0 graphical session and a valid binding".into()});
                        return;
                    }
                    let _ = send(&mut stream, &Message::Hello);
                    let _ = stream.set_nonblocking(true);
                    let authorized = Arc::new(AtomicBool::new(true));
                    let weak = Arc::downgrade(&authorized);
                    let session = r.session.clone();
                    std::thread::spawn(move || loop {
                        std::thread::sleep(Duration::from_millis(100));
                        let Some(flag) = weak.upgrade() else { break };
                        flag.store(allowed(uid, &session), Ordering::Relaxed);
                    });
                    let _ = tx.send(Client {
                        stream,
                        uid,
                        request: r,
                        authorized,
                        started: Instant::now(),
                    });
                });
            }
            std::thread::sleep(Duration::from_millis(10));
        });
        let mut clients: Vec<Client> = vec![];
        let mut devices: Vec<Keyboard> = Vec::new();
        let start = Instant::now();
        let mut refresh = Instant::now();
        let mut emergency = false;
        let mut current: Option<(bool, Option<Binding>)> = None;
        loop {
            while let Ok(client) = rx.try_recv() {
                if client.request.mode == "probe" {
                    let inventory = scan_keyboards();
                    let mut stream = client.stream;
                    let _ = send(
                        &mut stream,
                        &Message::Status {
                            version: env!("CARGO_PKG_VERSION").to_string(),
                            keyboards: inventory,
                        },
                    );
                    continue;
                }
                if client.request.mode == "agent" {
                    clients.retain(|c| !(c.uid == client.uid && c.request.mode == "agent"));
                }
                println!(
                    "Accepted {} client from uid {} for session {}",
                    client.request.mode, client.uid, client.request.session
                );
                clients.push(client);
                emergency = false;
            }
            clients.retain_mut(|c| {
                let mut b = [0];
                match c.stream.read(&mut b) {
                    Ok(0) => false,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
                    _ => false,
                }
            });
            clients.retain_mut(|c| {
                if c.request.mode == "learn" && c.started.elapsed() > LEARN_TIMEOUT {
                    let _ = send(
                        &mut c.stream,
                        &Message::Error {
                            message: "Key detection timed out".into(),
                        },
                    );
                    false
                } else {
                    true
                }
            });
            let learning = clients
                .iter()
                .position(|c| c.authorized.load(Ordering::Relaxed) && c.request.mode == "learn");
            let active = clients.iter().position(|c| {
                c.authorized.load(Ordering::Relaxed)
                    && c.request.mode == "agent"
                    && c.request.binding.is_some()
            });
            let binding = active.and_then(|i| clients[i].request.binding.clone());
            let signature = Some((learning.is_some(), binding.clone()));
            let signature_changed = current != signature;
            if signature_changed {
                for k in &mut devices {
                    release(k)
                }
                devices.clear();
                current = signature;
            }
            if (signature_changed || refresh.elapsed() > Duration::from_secs(2))
                && (learning.is_some() || binding.is_some())
            {
                let previous_count = devices.len();
                devices.retain(|keyboard| std::path::Path::new(&keyboard.path).exists());
                let known = devices
                    .iter()
                    .map(|keyboard| keyboard.path.clone())
                    .collect();
                devices.extend(keyboards(
                    &known,
                    if learning.is_some() {
                        None
                    } else {
                        binding.as_ref().map(|item| item.device.as_str())
                    },
                ));
                if signature_changed || devices.len() != previous_count {
                    println!(
                        "{} keyboard input node(s) opened for {}",
                        devices.len(),
                        if learning.is_some() {
                            "key detection"
                        } else {
                            "the active mapping"
                        }
                    );
                }
                refresh = Instant::now();
            }
            if devices.is_empty() && learning.is_some() {
                clients.retain_mut(|client| {
                    if client.request.mode == "learn" {
                        let _ = send(
                            &mut client.stream,
                            &Message::Error {
                                message: "No supported keyboard input was found. Check /dev/input, /dev/uinput, and the service log.".into(),
                            },
                        );
                        false
                    } else {
                        true
                    }
                });
            }
            if clients.is_empty() {
                devices.clear();
            }
            let matching = binding
                .as_ref()
                .map(|b| devices.iter().filter(|k| k.id == b.device).count())
                .unwrap_or(0);
            for k in &mut devices {
                let should = !emergency
                    && (learning.is_some()
                        || (matching == 1 && binding.as_ref().is_some_and(|b| b.device == k.id)));
                if should && !k.grabbed {
                    if !k
                        .dev
                        .get_key_state()
                        .map(|s| s.iter().next().is_none())
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    if let Err(error) = k.dev.grab() {
                        eprintln!("Could not grab {} ({}): {error}", k.name, k.path);
                        continue;
                    }
                    println!("Grabbed {} ({})", k.name, k.path);
                    k.grabbed = true;
                    k.learner = Learner::default();
                }
                if !should && k.grabbed {
                    release(k)
                }
                if !k.grabbed {
                    let _ = k.dev.fetch_events().map(|e| e.count());
                    continue;
                }
                if learning.is_none() && k.matcher.is_none() {
                    if let Some(b) = &binding {
                        k.matcher = Some(Matcher::new(b.clone()));
                    }
                }
                let fetched = k.dev.fetch_events().map(|e| e.collect::<Vec<_>>());
                let events = match fetched {
                    Ok(e) => e,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => vec![],
                    Err(_) => {
                        release(k);
                        continue;
                    }
                };
                let now = start.elapsed().as_millis() as u64;
                if learning.is_none() {
                    if let Some(m) = &mut k.matcher {
                        let o = m.tick(now);
                        emit(k, o.forward)?;
                    }
                }
                for event in events {
                    if event.event_type() != EventType::KEY {
                        if matches!(
                            event.event_type(),
                            EventType::RELATIVE
                                | EventType::ABSOLUTE
                                | EventType::MISC
                                | EventType::SWITCH
                        ) {
                            k.virtual_dev.emit(&[event])?;
                        }
                        continue;
                    }
                    let e = KeyEvent {
                        code: event.code(),
                        value: event.value(),
                    };
                    if let Some(i) = learning {
                        if e.value == 1 {
                            k.down.insert(e.code);
                        } else if e.value == 0 {
                            k.down.remove(&e.code);
                        }
                        if [14, 1, 28].iter().all(|code| k.down.contains(code)) {
                            let _ = send(
                                &mut clients[i].stream,
                                &Message::Error {
                                    message: "Detection cancelled by emergency shortcut".into(),
                                },
                            );
                            emergency = true;
                            break;
                        }
                        // During the short detection window keyboard events are consumed; timeout and emergency remain available.
                        if let Some(result) = k.learner.event(e, now) {
                            match result {
                                Ok((keys, trigger)) => {
                                    let b = Binding {
                                        device: k.id.clone(),
                                        keys,
                                        trigger,
                                    };
                                    println!("Detected key chord {:?} on {}", b.keys, k.name);
                                    let _ = send(
                                        &mut clients[i].stream,
                                        &Message::Learned {
                                            binding: b,
                                            name: k.name.clone(),
                                        },
                                    );
                                }
                                Err(message) => {
                                    eprintln!("Rejected key candidate on {}: {message}", k.name);
                                    let _ =
                                        send(&mut clients[i].stream, &Message::Error { message });
                                }
                            }
                        }
                    } else if let Some(m) = &mut k.matcher {
                        let out = m.event(e, now);
                        emit(k, out.forward)?;
                        if out.emergency {
                            emergency = true;
                            if let Some(i) = active {
                                let _ = send(&mut clients[i].stream, &Message::Emergency);
                            }
                            break;
                        }
                        if out.triggered {
                            println!("Triggered active mapping from {} ({})", k.name, k.path);
                            if let Some(i) = active {
                                if clients[i].authorized.load(Ordering::Relaxed) {
                                    let _ = send(
                                        &mut clients[i].stream,
                                        &Message::Trigger {
                                            sent_at_ms: Some(unix_millis()),
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }
            if emergency {
                for k in &mut devices {
                    release(k)
                }
            }
            std::thread::sleep(Duration::from_millis(if clients.is_empty() {
                20
            } else {
                3
            }));
        }
    }
    fn scan_keyboards() -> Vec<DeviceInfo> {
        evdev::enumerate()
            .filter_map(|(path, dev)| {
                let name = dev.name().unwrap_or("Keyboard").to_string();
                if name.starts_with("Coconut ") {
                    return None;
                }
                let keys = dev.supported_keys()?;
                if !keys.contains(KeyCode::KEY_A)
                    && !keys.contains(KeyCode::KEY_F23)
                    && !keys.contains(KeyCode::KEY_ASSISTANT)
                {
                    return None;
                }
                let input = dev.input_id();
                let mut copilot_keys = Vec::new();
                if keys.contains(KeyCode::KEY_F23) {
                    copilot_keys.push("F23".to_string());
                }
                if keys.contains(KeyCode::KEY_ASSISTANT) {
                    copilot_keys.push("Assistant".to_string());
                }
                Some(DeviceInfo {
                    id: format!(
                        "{:04x}:{:04x}:{}:{}",
                        input.vendor(),
                        input.product(),
                        dev.physical_path().unwrap_or(""),
                        name
                    ),
                    name,
                    path: path.to_string_lossy().into_owned(),
                    composite: dev.supported_relative_axes().is_some()
                        || dev.supported_absolute_axes().is_some(),
                    copilot_keys,
                })
            })
            .collect()
    }

    pub fn probe() -> Result<Vec<DeviceInfo>> {
        let stream = connect("probe", None)?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                bail!("Input service disconnected during health check")
            }
            match serde_json::from_str::<Message>(&line)? {
                Message::Status { keyboards, .. } => return Ok(keyboards),
                Message::Error { message } => bail!("{message}"),
                _ => {}
            }
        }
    }
    pub fn self_test_input() -> Result<()> {
        if unsafe { libc::geteuid() } != 0 {
            bail!("Input self-test source must run as root")
        }
        let mut keys = evdev::AttributeSet::<KeyCode>::new();
        for key in [
            KeyCode::KEY_A,
            KeyCode::KEY_LEFTSHIFT,
            KeyCode::KEY_LEFTMETA,
            KeyCode::KEY_F23,
        ] {
            keys.insert(key);
        }
        let mut device = VirtualDevice::builder()?
            .name("CNP Self Test Keyboard")
            .with_keys(&keys)?
            .build()?;
        println!("Synthetic keyboard ready; emitting two Copilot-style chords in 5 seconds");
        std::thread::sleep(Duration::from_secs(5));
        let chord = [
            KeyCode::KEY_LEFTSHIFT,
            KeyCode::KEY_LEFTMETA,
            KeyCode::KEY_F23,
        ];
        for attempt in 0..2 {
            device.emit(
                &chord
                    .iter()
                    .map(|key| InputEvent::new(EventType::KEY.0, key.code(), 1))
                    .collect::<Vec<_>>(),
            )?;
            std::thread::sleep(Duration::from_millis(20));
            device.emit(
                &chord
                    .iter()
                    .rev()
                    .map(|key| InputEvent::new(EventType::KEY.0, key.code(), 0))
                    .collect::<Vec<_>>(),
            )?;
            if attempt == 0 {
                std::thread::sleep(Duration::from_millis(300));
            }
        }
        std::thread::sleep(Duration::from_millis(500));
        println!("Synthetic keyboard test events emitted");
        Ok(())
    }
    pub fn learn_with_updates<F>(mut on_first: F) -> Result<Binding>
    where
        F: FnMut(&str, &Binding),
    {
        let stream = connect("learn", None)?;
        stream.set_read_timeout(Some(LEARN_TIMEOUT + Duration::from_secs(2)))?;
        let mut reader = BufReader::new(stream);
        let mut first: Option<Binding> = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                bail!("Input service disconnected")
            }
            match serde_json::from_str::<Message>(&line)? {
                Message::Learned { binding, name } => {
                    if first.as_ref() == Some(&binding) {
                        return Ok(binding);
                    }
                    on_first(&name, &binding);
                    first = Some(binding);
                }
                Message::Error { message } => bail!("{message}"),
                _ => {}
            }
        }
    }
    pub fn agent() -> Result<()> {
        use std::os::fd::AsRawFd;
        fs::create_dir_all(crate::config::state_dir())?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(crate::config::state_dir().join("agent.lock"))?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Ok(());
        }
        loop {
            let cfg = Config::load()?;
            if !cfg.enabled {
                return Ok(());
            }
            let fingerprint = fs::read(crate::config::config_path()).unwrap_or_default();
            let stream = match connect("agent", cfg.binding.clone()) {
                Ok(s) => s,
                Err(_) => {
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            stream.set_read_timeout(Some(Duration::from_millis(500)))?;
            let mut reader = BufReader::new(stream);
            loop {
                if fs::read(crate::config::config_path()).unwrap_or_default() != fingerprint {
                    break;
                }
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        continue
                    }
                    Err(_) => break,
                }
                match serde_json::from_str::<Message>(&line)? {
                    Message::Trigger { sent_at_ms } => {
                        if !allowed(unsafe { libc::geteuid() }, &session()?) {
                            continue;
                        }
                        let input_to_agent_ms =
                            sent_at_ms.map(|sent| unix_millis().saturating_sub(sent));
                        let launch_started = Instant::now();
                        match runner::run_active(&cfg) {
                            Ok(()) => {
                                let launch_request_ms: u64 = launch_started
                                    .elapsed()
                                    .as_millis()
                                    .try_into()
                                    .unwrap_or(u64::MAX);
                                let record = serde_json::json!({
                                    "recorded_at_ms": unix_millis(),
                                    "input_to_agent_ms": input_to_agent_ms,
                                    "launch_request_ms": launch_request_ms,
                                });
                                if let Ok(bytes) = serde_json::to_vec_pretty(&record) {
                                    let _ = crate::config::atomic_write(
                                        &crate::config::state_dir().join("last-trigger.json"),
                                        &bytes,
                                    );
                                }
                                if let Some(input_to_agent_ms) = input_to_agent_ms {
                                    eprintln!(
                                        "Coconut timing: input to agent {input_to_agent_ms} ms; launch request {launch_request_ms} ms."
                                    );
                                }
                                platform::notify(&cfg, "Action launched", false)
                            }
                            Err(e) => {
                                eprintln!("{e:#}");
                                platform::notify(&cfg, &e.to_string(), true);
                            }
                        }
                    }
                    Message::Emergency => {
                        let mut c = Config::load()?;
                        c.enabled = false;
                        c.save()?;
                        platform::notify(&c, "Input mapping paused by emergency shortcut", true);
                        break;
                    }
                    Message::Error { message } => {
                        eprintln!("{message}");
                        std::thread::sleep(Duration::from_secs(2));
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
}
#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "windows")]
#[path = "windows_service.rs"]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub composite: bool,
    pub copilot_keys: Vec<String>,
}
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn run() -> anyhow::Result<()> {
    anyhow::bail!("Input service is Linux-only")
}
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn agent() -> anyhow::Result<()> {
    anyhow::bail!("Input agent is Linux-only")
}
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn learn_with_updates<F>(_on_first: F) -> anyhow::Result<crate::config::Binding>
where
    F: FnMut(&str, &crate::config::Binding),
{
    anyhow::bail!("Key detection is Linux-only")
}
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn probe() -> anyhow::Result<Vec<DeviceInfo>> {
    anyhow::bail!("Input service is Linux-only")
}
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn self_test_input() -> anyhow::Result<()> {
    anyhow::bail!("Input self-test source is Linux-only")
}

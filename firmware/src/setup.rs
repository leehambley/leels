//! WiFi setup mode, started from the Setup button on the display.
//!
//! Brings up a WPA2 hotspot with:
//! - DHCP that hands out addresses, names us as DNS server and advertises the
//!   captive-portal URL (RFC 8910);
//! - DNS that answers every name with our address;
//! - the settings page on port 80. Any other path redirects to it, which is
//!   what phones look for to pop up the portal.
//!
//! Saving hands the settings to the UI task (which owns flash and knows
//! whether the carriage is idle), then the controller restarts with WiFi off.

use core::cell::Cell;
use core::fmt::Write as _;
use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use els_core::settings::Settings;
use els_core::web::{self, Method};
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::{Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use embedded_io_async::Write;
use esp_hal::peripherals::WIFI;
use esp_println::println;
use esp_radio::wifi::{AccessPointConfig, AuthMethod, ModeConfig, WifiDevice};
use static_cell::StaticCell;

use crate::config;

/// UI → setup: bring up the hotspot.
pub static START: Signal<CriticalSectionRawMutex, ()> = Signal::new();
/// Web page → UI: save these settings. The UI answers on [`SAVE_RESULT`].
pub static SAVE_REQUEST: Channel<CriticalSectionRawMutex, Settings, 1> = Channel::new();
pub static SAVE_RESULT: Signal<CriticalSectionRawMutex, Result<(), &'static str>> = Signal::new();
/// Settings the page shows and edits (the ones currently in use).
pub static CURRENT: Mutex<CriticalSectionRawMutex, Cell<Option<Settings>>> = Mutex::new(Cell::new(None));

const HTTP_WORKERS: usize = 3;
/// TCP sockets for HTTP, UDP sockets for DHCP and DNS, plus headroom.
const SOCKETS: usize = HTTP_WORKERS + 2 + 3;

#[embassy_executor::task]
pub async fn setup_task(spawner: Spawner, wifi: WIFI<'static>) {
    START.wait().await;
    println!("setup: starting hotspot {}", config::SETUP_SSID);

    static RADIO: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
    let radio = RADIO.init(esp_radio::init().expect("radio init"));
    let (mut controller, interfaces) = esp_radio::wifi::new(radio, wifi, Default::default()).expect("wifi init");

    let ip = Ipv4Addr::from(config::SETUP_IP);
    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(ip, 24),
        gateway: Some(ip),
        dns_servers: Default::default(),
    });
    let rng = esp_hal::rng::Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    static RESOURCES: StaticCell<StackResources<SOCKETS>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(interfaces.ap, net_config, RESOURCES.init(StackResources::new()), seed);

    let ap = AccessPointConfig::default()
        .with_ssid(config::SETUP_SSID.into())
        .with_password(config::SETUP_PASSWORD.into())
        .with_auth_method(AuthMethod::Wpa2Personal);
    controller.set_config(&ModeConfig::AccessPoint(ap)).expect("wifi config");
    controller.start_async().await.expect("wifi start");

    spawner.spawn(net_task(runner)).unwrap();
    spawner.spawn(dhcp_task(stack)).unwrap();
    spawner.spawn(dns_task(stack)).unwrap();
    for _ in 0..HTTP_WORKERS {
        spawner.spawn(http_task(stack)).unwrap();
    }
    println!("setup: open {}", config::SETUP_URL);
    // Keep the controller alive; leaving setup restarts the chip.
    core::future::pending::<()>().await;
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
async fn dhcp_task(stack: Stack<'static>) {
    use edge_dhcp::io::{self, DEFAULT_SERVER_PORT};
    use edge_dhcp::server::{Server, ServerOptions};
    use edge_nal::UdpBind;
    use edge_nal_embassy::{Udp, UdpBuffers};

    let ip = Ipv4Addr::from(config::SETUP_IP);
    let dns = [ip];
    let mut gateway = [Ipv4Addr::UNSPECIFIED];
    let mut options = ServerOptions::new(ip, Some(&mut gateway));
    options.dns = &dns;
    options.captive_url = Some(config::SETUP_URL);

    let buffers = UdpBuffers::<1, 1024, 1024, 4>::new();
    let udp = Udp::new(stack, &buffers);
    let mut socket = udp
        .bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DEFAULT_SERVER_PORT)))
        .await
        .expect("dhcp bind");
    let mut buf = [0u8; 1500];
    loop {
        if let Err(e) = io::server::run(&mut Server::<_, 8>::new_with_et(ip), &options, &mut socket, &mut buf).await {
            println!("setup: dhcp error {:?}", e);
        }
        Timer::after_millis(500).await;
    }
}

#[embassy_executor::task]
async fn dns_task(stack: Stack<'static>) {
    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_buf = [0u8; 1024];
    let mut tx_buf = [0u8; 1024];
    let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
    socket.bind(53).expect("dns bind");
    let mut query = [0u8; 512];
    let mut reply = [0u8; 512];
    loop {
        let Ok((n, meta)) = socket.recv_from(&mut query).await else { continue };
        if let Some(len) = els_core::dns::answer(&query[..n], config::SETUP_IP, &mut reply) {
            let _ = socket.send_to(&reply[..len], meta).await;
        }
    }
}

type Page = heapless::String<12_288>;

#[embassy_executor::task(pool_size = HTTP_WORKERS)]
async fn http_task(stack: Stack<'static>) {
    let mut rx = [0u8; 2048];
    let mut tx = [0u8; 2048];
    let mut request = [0u8; 2048];
    let mut page = Page::new();
    loop {
        let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
        socket.set_timeout(Some(Duration::from_secs(10)));
        if socket.accept(80).await.is_err() {
            continue;
        }
        if let Err(e) = serve(&mut socket, &mut request, &mut page).await {
            println!("setup: http error {:?}", e);
        }
        let _ = socket.flush().await;
        socket.close();
        Timer::after_millis(50).await;
        socket.abort();
    }
}

async fn serve(socket: &mut TcpSocket<'_>, request: &mut [u8], page: &mut Page) -> Result<(), embassy_net::tcp::Error> {
    // Read until headers and body are complete.
    let mut n = 0;
    let (header_len, content_length) = loop {
        let read = socket.read(&mut request[n..]).await?;
        if read == 0 {
            return Ok(());
        }
        n += read;
        if let Some(r) = web::parse_request(&request[..n]) {
            if r.header_len + r.content_length <= n {
                break (r.header_len, r.content_length);
            }
            if r.header_len + r.content_length > request.len() {
                return respond(socket, "413 Payload Too Large", "text/plain", b"too large").await;
            }
        } else if n == request.len() {
            return respond(socket, "431 Request Header Fields Too Large", "text/plain", b"too large").await;
        }
    };
    let Some(req) = web::parse_request(&request[..n]) else { return Ok(()) };
    let current = CURRENT.lock(|c| c.get()).unwrap_or(config::DEFAULTS);

    match (req.method, req.path) {
        (Method::Get, "/") => render(socket, page, &current, None).await,
        (Method::Post, "/save") => {
            let body = core::str::from_utf8(&request[header_len..header_len + content_length]).unwrap_or("");
            match web::apply_form(body, &current, config::TIMING) {
                Err(e) => render(socket, page, &current, Some(e.message())).await,
                Ok(new) => {
                    SAVE_RESULT.reset();
                    SAVE_REQUEST.send(new).await;
                    match SAVE_RESULT.wait().await {
                        Ok(()) => {
                            respond(socket, "200 OK", "text/html; charset=utf-8", web::SAVED_PAGE.as_bytes()).await
                        }
                        Err(e) => render(socket, page, &current, Some(e)).await,
                    }
                }
            }
        }
        // Captive-portal probes and anything else: send the browser to the page.
        _ => {
            let mut head = heapless::String::<160>::new();
            let _ = write!(
                head,
                "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                config::SETUP_URL
            );
            socket.write_all(head.as_bytes()).await
        }
    }
}

async fn render(
    socket: &mut TcpSocket<'_>,
    page: &mut Page,
    settings: &Settings,
    notice: Option<&str>,
) -> Result<(), embassy_net::tcp::Error> {
    page.clear();
    let _ = web::render_page(page, settings, notice);
    respond(socket, "200 OK", "text/html; charset=utf-8", page.as_bytes()).await
}

async fn respond(
    socket: &mut TcpSocket<'_>,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), embassy_net::tcp::Error> {
    let mut head = heapless::String::<192>::new();
    let _ = write!(
        head,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(body).await
}

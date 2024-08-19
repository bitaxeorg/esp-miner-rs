#![no_std]
#![no_main]

use core::str::FromStr;
use embassy_executor::Spawner;
use embassy_net::{tcp::TcpSocket, Config, Ipv4Address, Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Ticker, Timer};
use esp_backtrace as _;
use esp_hal::{
    clock::ClockControl,
    peripherals::Peripherals,
    prelude::*,
    rng::Rng,
    system::SystemControl,
    timer::{timg::TimerGroup, ErasedTimer, OneShotTimer, PeriodicTimer},
};
use esp_println::println;
use esp_wifi::{
    initialize,
    wifi::{
        ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiStaDevice,
        WifiState,
    },
    EspWifiInitFor,
};
use heapless::{String, Vec};
use static_cell::StaticCell;
use stratum_v1::{Client, Extensions, Message, Share, VersionRolling};

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

#[main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let peripherals = Peripherals::take();

    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::max(system.clock_control).freeze();

    let timer = PeriodicTimer::new(
        TimerGroup::new(peripherals.TIMG1, &clocks, None)
            .timer0
            .into(),
    );
    let init = initialize(
        EspWifiInitFor::Wifi,
        timer,
        Rng::new(peripherals.RNG),
        peripherals.RADIO_CLK,
        &clocks,
    )
    .unwrap();

    let wifi = peripherals.WIFI;
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiStaDevice).unwrap();

    let systimer = esp_hal::timer::systimer::SystemTimer::new(peripherals.SYSTIMER);
    let alarm0: ErasedTimer = systimer.alarm0.into();
    let timers = [OneShotTimer::new(alarm0)];
    let timers = mk_static!([OneShotTimer<ErasedTimer>; 1], timers);
    esp_hal_embassy::init(&clocks, timers);

    let config = Config::dhcpv4(Default::default());

    let seed = 1234; // very random, very secure seed

    // Init network stack
    let stack = &*mk_static!(
        Stack<WifiDevice<'_, WifiStaDevice>>,
        Stack::new(
            wifi_interface,
            config,
            mk_static!(StackResources<3>, StackResources::<3>::new()),
            seed
        )
    );

    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(stack)).ok();

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    println!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            println!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    loop {
        Timer::after(Duration::from_millis(1_000)).await;

        let mut socket = TcpSocket::new(
            stack,
            mk_static!([u8; 1536], [0; 1536]),
            mk_static!([u8; 1536], [0; 1536]),
        );
        socket.set_timeout(Some(embassy_time::Duration::from_secs(3)));

        let remote_endpoint = (Ipv4Address::new(68, 235, 52, 36), 21496); // public-pool
        println!("connecting...");
        let r = socket.connect(remote_endpoint).await;
        if let Err(e) = r {
            println!("connect error: {:?}", e);
            continue;
        }
        println!("connected!");

        let mut client = Client::<_, 1480, 512>::new(socket);
        client.enable_software_rolling(false, true, false);

        let client = mk_static!(
            Mutex::<NoopRawMutex, Client<TcpSocket, 1480, 512>>,
            Mutex::<NoopRawMutex, _>::new(client)
        );

        static AUTH_SIGNAL: StaticCell<Signal<NoopRawMutex, bool>> = StaticCell::new();
        let auth_signal = &*AUTH_SIGNAL.init(Signal::new());

        spawner.spawn(stratum_v1_rx_task(client, auth_signal)).ok();
        spawner.spawn(stratum_v1_tx_task(client, auth_signal)).ok();

        let exts = Extensions {
            version_rolling: Some(VersionRolling {
                mask: Some(0x1fffe000),
                min_bit_count: Some(16),
            }),
            minimum_difficulty: Some(256),
            subscribe_extranonce: None,
            info: None,
        };
        let mut client = client.lock().await;
        client.send_configure(exts).await.unwrap();
        loop {
            Timer::after(Duration::from_millis(5_000)).await;
        }
    }
}

#[embassy_executor::task]
async fn stratum_v1_rx_task(
    client: &'static Mutex<NoopRawMutex, Client<TcpSocket<'static>, 1480, 512>>,
    authorized: &'static Signal<NoopRawMutex, bool>,
) {
    // let mut ticker = Ticker::every(Duration::from_secs(1));
    loop {
        // ticker.next().await;
        let mut client = client.lock().await;
        match client.poll_message().await {
            Ok(msg) => {
                if let Some(msg) = msg {
                    match msg {
                        Message::Configured => {
                            client
                                .send_connect(Some(String::<32>::from_str("esp-miner-rs").unwrap()))
                                .await
                                .unwrap();
                        }
                        Message::Connected => {
                            client
                                .send_authorize(
                                    String::<64>::from_str(
                                        "1HLQGxzAQWnLore3fWHc2W8UP1CgMv1GKQ.miner1",
                                    )
                                    .unwrap(),
                                    String::<64>::from_str("x").unwrap(),
                                )
                                .await
                                .unwrap();
                        }
                        Message::Authorized => {
                            authorized.signal(true);
                        }
                        Message::Share {
                            accepted: _,
                            rejected: _,
                        } => {
                            // TODO update the display/statistics if any
                        }
                        Message::VersionMask(_mask) => {
                            // TODO use mask for hardware version rolling is available
                        }
                        Message::Difficulty(_diff) => {
                            // TODO use diff to filter ASIC reported hits
                        }
                        Message::CleanJobs => {
                            // TODO clean the job queue and immediately start hashing a new job
                        }
                    }
                }
            }
            Err(e) => {
                println!("Client receive_message error: {:?}", e);
            }
        }
    }
}

#[embassy_executor::task]
async fn stratum_v1_tx_task(
    client: &'static Mutex<NoopRawMutex, Client<TcpSocket<'static>, 1480, 512>>,
    authorized: &'static Signal<NoopRawMutex, bool>,
) {
    // use the Signal from stratum_v1_rx_task to start looking for shares
    while !authorized.wait().await {
        authorized.reset();
        Timer::after(Duration::from_millis(500)).await;
    }
    let mut ticker = Ticker::every(Duration::from_secs(2));
    loop {
        ticker.next().await;
        // TODO implement a channel to receive real shares from the ASIC, for now send fake shares periodically
        let mut client = client.lock().await;
        let mut extranonce2 = Vec::new();
        extranonce2.resize(4, 0).unwrap();
        extranonce2[3] = 0x01;
        let fake_share = Share {
            job_id: String::<64>::from_str("01").unwrap(), // TODO will come from the Job
            extranonce2,                                   // TODO will come from the Job
            ntime: 1722789905,                             // TODO will come from the Job
            nonce: 0,                                      // TODO will come from the ASIC hit
            version_bits: None, // TODO will come from the ASIC hit if hardware version rolling is enabled
        };
        client.send_submit(fake_share).await.unwrap();
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");
    println!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        if esp_wifi::wifi::get_wifi_state() == WifiState::StaConnected {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5000)).await
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: SSID.try_into().unwrap(),
                password: PASSWORD.try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            println!("Starting wifi");
            controller.start().await.unwrap();
            println!("Wifi started!");
        }
        println!("About to connect...");

        match controller.connect().await {
            Ok(_) => println!("Wifi connected!"),
            Err(e) => {
                println!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    stack.run().await
}

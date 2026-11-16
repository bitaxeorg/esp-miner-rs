#![no_std]
#![no_main]

mod io06;
// mod power;
mod variants;

use core::{net::Ipv4Addr, str::FromStr};
use defmt::{debug, error, info, trace, Debug2Format};
use defmt_rtt as _;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_net::{tcp::TcpSocket, Runner, StackResources};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Ticker, Timer};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{clock::CpuClock, i2c::master::I2c, rng::Rng, time::Rate, timer::timg::TimerGroup};
use esp_radio::wifi::{
    sta::StationConfig, AuthenticationMethodConfig, Config, ControllerConfig, Interface,
    WifiController,
};
use heapless::{String, Vec};
use io06::Io06;
use static_cell::StaticCell;
use stratum_v1::{Client, Extensions, Message, Share, VersionRolling};

esp_bootloader_esp_idf::esp_app_desc!();

type StratumClient = Client<Io06<TcpSocket<'static>>, 1480, 512>;

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    static I2C_BUS: StaticCell<Mutex<NoopRawMutex, I2c<'_, esp_hal::Async>>> = StaticCell::new();
    let i2c = I2c::new(
        peripherals.I2C0,
        esp_hal::i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
    )
    .unwrap()
    .with_sda(peripherals.GPIO47)
    .with_scl(peripherals.GPIO48)
    .into_async();
    let i2c_bus = I2C_BUS.init(Mutex::new(i2c));

    // TODO: this EMC2101 should be abstracted by AsicTemp and SetFan trait
    #[cfg(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))]
    let mut _emc2101 = emc2101::AsyncEMC2101::new(I2cDevice::new(i2c_bus))
        .await
        .unwrap();

    #[cfg(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))]
    let _ds4432 =
        ds4432::AsyncDS4432::with_rfs(I2cDevice::new(i2c_bus), Some(80_000), None).unwrap(); // On BitaxeMax/Ultra, Rfs0 = 80k, Rfs1 = DNP

    #[cfg(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))]
    let _ina260 = ina260::AsyncINA260::new(I2cDevice::new(i2c_bus))
        .await
        .unwrap();

    #[cfg(feature = "bitaxe-max")]
    let _vcore_target_mv = 1.4; // TODO let user choice the target VCore / HashFreq
    #[cfg(feature = "bitaxe-ultra")]
    let _vcore_target_mv = 1.2; // TODO let user choice the target VCore / HashFreq

    // #[cfg(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))]
    // spawner
    //     .spawn(power::vcore_task(vcore_target_mv, ds4432, Some(ina260)))
    //     .ok();

    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(WIFI_SSID.try_into().unwrap())
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                WIFI_PASSWORD.try_into().unwrap(),
            )),
    );
    let wifi_interface = Interface::station();
    let controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .unwrap();

    let config = embassy_net::Config::dhcpv4(Default::default());

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    spawner.spawn(connection_task(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    info!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            info!("Got IP: {}", config.address);
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

        let remote_endpoint = (Ipv4Addr::new(68, 235, 52, 36), 21496); // public-pool
        info!("connecting to pool...");
        let r = socket.connect(remote_endpoint).await;
        if let Err(e) = r {
            error!("connect error: {:?}", e);
            continue;
        }
        info!("connected!");

        let mut client = StratumClient::new(Io06(socket));
        client.enable_software_rolling(false, true, false);

        let client = mk_static!(
            Mutex::<NoopRawMutex, StratumClient>,
            Mutex::<NoopRawMutex, _>::new(client)
        );

        static AUTH_SIGNAL: StaticCell<Signal<NoopRawMutex, bool>> = StaticCell::new();
        let auth_signal = &*AUTH_SIGNAL.init(Signal::new());

        spawner.spawn(stratum_v1_rx_task(client, auth_signal).unwrap());
        spawner.spawn(stratum_v1_tx_task(client, auth_signal).unwrap());

        let exts = Extensions {
            version_rolling: Some(VersionRolling {
                mask: Some(0x1fffe000),
                min_bit_count: Some(16),
            }),
            minimum_difficulty: Some(256),
            subscribe_extranonce: None,
            info: None,
        };
        {
            let mut client = client.lock().await;
            client.send_configure(exts).await.unwrap();
        }
        loop {
            Timer::after(Duration::from_millis(5_000)).await;
        }
    }
}

#[embassy_executor::task]
async fn stratum_v1_rx_task(
    client: &'static Mutex<NoopRawMutex, StratumClient>,
    authorized: &'static Signal<NoopRawMutex, bool>,
) {
    debug!("start stratum v1 RX task");
    // let mut ticker = Ticker::every(Duration::from_secs(1));
    loop {
        // ticker.next().await;
        let mut client = client.lock().await;
        trace!("Polling message...");
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
                error!("Client receive_message error: {:?}", Debug2Format(&e));
            }
        }
    }
}

#[embassy_executor::task]
async fn stratum_v1_tx_task(
    client: &'static Mutex<NoopRawMutex, StratumClient>,
    authorized: &'static Signal<NoopRawMutex, bool>,
) {
    debug!("start stratum v1 TX task");
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
async fn connection_task(mut controller: WifiController<'static>) {
    debug!("start connection task");
    loop {
        debug!("About to connect...");

        match controller.connect_async().await {
            Ok(_) => {
                debug!("Wifi connected!");
                // wait until we're no longer connected
                controller.wait_for_disconnect_async().await.ok();
                debug!("Wifi disconnected");
            }
            Err(e) => {
                error!("Failed to connect to wifi: {:?}", e);
            }
        }
        Timer::after(Duration::from_millis(5000)).await
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}

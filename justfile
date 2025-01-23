set export

WIFI_SSID := "my_ssid"
WIFI_PASSWORD := "my_password"

build b:
  cargo build --release --features=bitaxe-$b

run b:
  cargo run --release --features=bitaxe-$b

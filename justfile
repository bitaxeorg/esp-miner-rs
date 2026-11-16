set export

WIFI_SSID := "my_ssid"
WIFI_PASSWORD := "my_password"

build b:
  cd firmware && cargo build --release --features=bitaxe-$b

run b:
  cd firmware && cargo run --release --features=bitaxe-$b

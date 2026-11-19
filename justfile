set export

WIFI_SSID := "my_ssid"
WIFI_PASSWORD := "my_password"

build b:
  cd firmware && cargo build --release --features=bitaxe-$b

run b:
  cd firmware && cargo run --release --features=bitaxe-$b

test:
  cargo test

# regenerate the display fonts and images from their LVGL sources
display-assets sources unscii:
  cargo run -p display-assets -- $sources $unscii

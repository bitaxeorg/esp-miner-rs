//! Which status screen to show.

/// A status screen, in carousel order from [`Screen::Connection`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Overheat,
    Connection,
    BitaxeLogo,
    OsmuLogo,
    Urls,
    Stats,
    Mining,
    Wifi,
}

impl Screen {
    /// Time on the screen before the next one (ms).
    fn delay_ms(self) -> u32 {
        match self {
            Screen::Overheat => 0,
            Screen::Connection => 1_000,
            Screen::BitaxeLogo | Screen::OsmuLogo => 3_000,
            Screen::Urls | Screen::Stats | Screen::Mining | Screen::Wifi => 10_000,
        }
    }

    /// The screen after this one: the carousel loops from [`Screen::Urls`] to [`Screen::Wifi`].
    fn next(self) -> Screen {
        match self {
            Screen::Overheat | Screen::Connection => Screen::BitaxeLogo,
            Screen::BitaxeLogo => Screen::OsmuLogo,
            Screen::OsmuLogo => Screen::Urls,
            Screen::Urls => Screen::Stats,
            Screen::Stats => Screen::Mining,
            Screen::Mining => Screen::Wifi,
            Screen::Wifi => Screen::Urls,
        }
    }
}

/// The screen shown, from the miner state, the time and the button.
///
/// The overheat screen stays once shown. Without network, the connection screen. With it, the
/// connection screen a second more, then the Bitaxe and OSMU logos, then the URLs, statistics,
/// mining and Wi-Fi screens in a loop, each for its delay or until the button. A large display
/// shows the statistics screen alone once connected.
pub struct Carousel {
    screen: Screen,
    /// Time on the current screen while connected (ms).
    elapsed_ms: u32,
    large: bool,
}

impl Carousel {
    pub fn new(large: bool) -> Self {
        Self {
            screen: Screen::Connection,
            elapsed_ms: 0,
            large,
        }
    }

    /// The screen to show `dt_ms` after the last update.
    pub fn update(&mut self, dt_ms: u32, connected: bool, overheat: bool) -> Screen {
        if overheat || self.screen == Screen::Overheat {
            self.show(Screen::Overheat);
        } else if !connected {
            self.show(Screen::Connection);
        } else if self.large {
            self.show(Screen::Stats);
        } else {
            self.elapsed_ms = self.elapsed_ms.saturating_add(dt_ms);
            if self.elapsed_ms > self.screen.delay_ms() {
                self.show(self.screen.next());
            }
        }
        self.screen
    }

    /// The button: the next screen of the carousel, or the statistics on a large display.
    pub fn button(&mut self) -> Screen {
        match self.screen {
            Screen::Overheat => {}
            _ if self.large => self.show(Screen::Stats),
            screen => self.show(screen.next()),
        }
        self.screen
    }

    /// Time on the current screen while connected (ms).
    pub fn elapsed_ms(&self) -> u32 {
        self.elapsed_ms
    }

    fn show(&mut self, screen: Screen) {
        if self.screen != screen {
            self.screen = screen;
            self.elapsed_ms = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_until_the_network_is_up() {
        let mut carousel = Carousel::new(false);
        assert_eq!(carousel.update(60_000, false, false), Screen::Connection);
        // a second more once connected
        assert_eq!(carousel.update(1_000, true, false), Screen::Connection);
        assert_eq!(carousel.update(500, true, false), Screen::BitaxeLogo);
    }

    #[test]
    fn logos_then_a_loop() {
        let mut carousel = Carousel::new(false);
        carousel.update(1_500, true, false);
        let mut seen = [Screen::Connection; 8];
        for screen in seen.iter_mut() {
            // 500 ms steps, as the display refreshes
            loop {
                let shown = carousel.update(500, true, false);
                if carousel.elapsed_ms() == 0 {
                    *screen = shown;
                    break;
                }
            }
        }
        assert_eq!(
            seen,
            [
                Screen::OsmuLogo,
                Screen::Urls,
                Screen::Stats,
                Screen::Mining,
                Screen::Wifi,
                Screen::Urls,
                Screen::Stats,
                Screen::Mining,
            ]
        );
    }

    #[test]
    fn the_button_moves_on() {
        let mut carousel = Carousel::new(false);
        carousel.update(0, true, false);
        assert_eq!(carousel.button(), Screen::BitaxeLogo);
        assert_eq!(carousel.button(), Screen::OsmuLogo);
        assert_eq!(carousel.button(), Screen::Urls);
        assert_eq!(carousel.update(500, true, false), Screen::Urls);
    }

    #[test]
    fn large_displays_show_the_statistics() {
        let mut carousel = Carousel::new(true);
        assert_eq!(carousel.update(500, false, false), Screen::Connection);
        assert_eq!(carousel.update(500, true, false), Screen::Stats);
        assert_eq!(carousel.update(60_000, true, false), Screen::Stats);
        assert_eq!(carousel.button(), Screen::Stats);
    }

    #[test]
    fn overheat_stays() {
        let mut carousel = Carousel::new(false);
        carousel.update(500, true, false);
        assert_eq!(carousel.update(500, true, true), Screen::Overheat);
        assert_eq!(carousel.update(500, true, false), Screen::Overheat);
        assert_eq!(carousel.button(), Screen::Overheat);
    }
}

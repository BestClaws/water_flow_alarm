

use embassy_futures::select::Either::{First, Second};
use embassy_futures::select::select;
use embassy_time::{Duration, Instant, Timer};
use embedded_hal_async::digital::Wait;
use embedded_hal::digital::InputPin;

pub struct Encoder<PIN> {
    last_high: Instant,
    last_state: bool,
    // pin: GpioPin<Input<PullDown>, 6>,
    pin: PIN,
}


impl<PIN> Encoder<PIN> where PIN: InputPin + Wait {
    pub fn new(pin: PIN) -> Self {
        Encoder  {
            pin,
            last_high: Instant::now(),
            last_state: false,

        }
    }

    /// better use get_val_avg(), as the current implementation of this
    /// records last_high - which helps calculating aangular velocity
    /// only when the method is called, so the later it is called, the lower
    /// and possibly lowest more often than not.
    async fn get_val(&mut self) -> u8 {

        let mut is_high: bool;
        let mut now: Instant;
        let mut difference: Duration;
        let mut difference_ms: u64;

        loop {
            self.pin.wait_for_any_edge().await.unwrap();
            is_high = self.pin.is_high().unwrap();
            now = Instant::now();
            difference = now - self.last_high;
            difference_ms = difference.as_millis();
            log::debug!("loop: last high: {}, now: {}", self.last_high, now);

            if difference_ms < 20 {
                continue;
            }
            if is_high {
                if self.last_state == false {
                    let answer = ((2000 / difference_ms) * 1) as u8;
                    unsafe { self.last_high = now; }
                    self.last_state = true;
                    return answer.max(1);
                }
            } else {
                unsafe { self.last_state = false; }
            }
        }
    }

    pub(crate) async fn get_val_avg(&mut self, n: u8) -> f64 {
        let mut sum: f64 = 0.0;
        for _ in 0..n {
            sum = sum + self.get_val().await as f64;
        }
        return sum / n as f64;
    }

    /// check weather encoder is active or not.
    pub(crate) async fn active(&mut self) -> bool {
        // return true;
        const TRIES: u64 = 3;
        const GAP_BETWEEN_TRIES: u64 =  500;
        const TOTAL_TEST_TIME: u64 = GAP_BETWEEN_TRIES * TRIES  * 2;
        match select((async move || {

            const TRIES: u64 = 3;
            const GAP_BETWEEN_TRIES: u64 =  500;
            const TOTAL_TEST_TIME: u64 = GAP_BETWEEN_TRIES * TRIES  * 2;
            for _ in 1..=TRIES {
                let _ = self.get_val().await;
                Timer::after_millis(GAP_BETWEEN_TRIES).await;
            }

        })(), Timer::after_millis(TOTAL_TEST_TIME)).await {
            First(x) => {
                true
            },
            Second(y) => {
                false
            }
        }
    }
    
    
    pub async fn get_voltage_level(&mut self) -> bool {
       if self.pin.is_high().unwrap() {
           true
       } else {
           false
       }
    } 

}
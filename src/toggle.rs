use tokio::sync::{Notify, RwLock};

pub struct Toggle {
  state: RwLock<bool>,
  notify_true: Notify,
  notify_false: Notify,
}

impl Toggle {
  pub fn new(state: bool) -> Toggle {
    Toggle {
      state: RwLock::new(state),
      notify_true: Notify::new(),
      notify_false: Notify::new(),
    }
  }

  pub async fn wait_for(&self, state: bool) {
    match state {
      true  => {self.notify_true.notified().await},
      false => {self.notify_false.notified().await},
    };
  }

  pub async fn set(&self, state: bool) {
    {
      let mut _state = self.state.write().await;
      if *_state == state {
        return;
      }
      *_state = state;
    }
    match state {
      true  => {self.notify_true.notify_waiters()},
      false => {self.notify_false.notify_waiters()},
    };
  }

  pub async fn get(&self) -> bool {
    let _state = self.state.read().await;
    *_state
  }
}

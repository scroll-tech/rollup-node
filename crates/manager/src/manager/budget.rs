/// The budget for the L1 notification channel.
pub(crate) const L1_NOTIFICATION_CHANNEL_BUDGET: u32 = 5;

/// Polls the given stream. Breaks with `true` if there maybe is more work.
#[macro_export]
macro_rules! poll_nested_stream_with_budget {
    ($target:literal, $label:literal, $budget:ident, $poll_stream:expr, $on_ready_some:expr $(, $on_ready_none:expr;)? $(,)?) => {{
        let mut budget: u32 = $budget;

            loop {
                match $poll_stream {
                    Poll::Ready(Some(item)) => {
                        $on_ready_some(item);

                        budget -= 1;
                        if budget == 0 {
                            break true
                        }
                    }
                    Poll::Ready(None) => {
                        $($on_ready_none;)? // todo: handle error case with $target and $label
                        break false
                    }
                    Poll::Pending => break false,
                }
            }
    }};
}

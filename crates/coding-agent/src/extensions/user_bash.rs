use pi_durable::cancel::CancelToken;
use serde_json::Value;

use super::Live;
use crate::bash_exec::Outcome;

pub enum UserBash {
    Local,
    Answered(Outcome),
    Ran(Result<Outcome, String>),
    Refused(String),
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Reply {
    Answered { result: Outcome },
    Ran { ran: Outcome },
    Refused { refused: String },
}

impl Live {
    pub fn user_bash(
        &self,
        event: Value,
        resolved: &str,
        abort: &CancelToken,
        started: &mut dyn FnMut(),
        chunk: &mut dyn FnMut(String),
    ) -> UserBash {
        if !self.handles("user_bash") {
            return UserBash::Local;
        }
        let pending = match self.begin(|host, ctx| host.user_bash(event, resolved, ctx)) {
            Ok(pending) => pending,
            Err(error) => return UserBash::Ran(Err(error)),
        };
        let exec = CancelToken::new();
        let mut link = None;
        let answer = pending.wait(Some(&exec), &mut |part| {
            if part.get("exec").is_some() {
                started();
                let exec = exec.clone();
                link = Some(abort.subscribe(move || exec.cancel()));
            } else if let Some(delta) = part.get("delta").and_then(Value::as_str) {
                chunk(delta.to_owned());
            }
        });
        drop(link);
        match answer {
            Err(error) => {
                self.died();
                UserBash::Ran(Err(error))
            }
            Ok(answer) => match serde_json::from_value::<Option<Reply>>(answer) {
                Ok(None) => UserBash::Local,
                Ok(Some(Reply::Answered { result })) => UserBash::Answered(result),
                Ok(Some(Reply::Ran { ran })) => UserBash::Ran(Ok(ran)),
                Ok(Some(Reply::Refused { refused })) => UserBash::Refused(refused),
                Err(error) => UserBash::Ran(Err(format!(
                    "the extension host answered user_bash with {error}"
                ))),
            },
        }
    }
}

use distant_net::{AuthChallengeFn, AuthErrorFn, AuthInfoFn, AuthVerifyFn, AuthVerifyKind};
use log::*;
use std::io;

/// Configuration to use when creating a new [`DistantManagerClient`](super::DistantManagerClient)
pub struct DistantManagerClientConfig {
    pub on_challenge: Box<AuthChallengeFn>,
    pub on_verify: Box<AuthVerifyFn>,
    pub on_info: Box<AuthInfoFn>,
    pub on_error: Box<AuthErrorFn>,
}

impl DistantManagerClientConfig {
    /// Creates a new config with prompts that return empty strings
    pub fn with_empty_prompts() -> Self {
        Self::with_prompts(|_| Ok("".to_string()), |_| Ok("".to_string()))
    }

    /// Creates a new config with two prompts
    ///
    /// * `password_prompt` - used for prompting for a secret, and should not display what is typed
    /// * `text_prompt` - used for general text, and is okay to display what is typed
    pub fn with_prompts<PP, PT>(password_prompt: PP, text_prompt: PT) -> Self
    where
        PP: Fn(&str) -> io::Result<String> + Send + Sync + 'static,
        PT: Fn(&str) -> io::Result<String> + Send + Sync + 'static,
    {
        Self {
            on_challenge: Box::new(move |questions, _extra| {
                trace!("[manager client] on_challenge({questions:?}, {_extra:?})");
                let mut answers = Vec::new();
                for question in questions.iter() {
                    // Contains all prompt lines including same line
                    let mut lines = question.text.split('\n').collect::<Vec<_>>();

                    // Line that is prompt on same line as answer
                    let line = lines.pop().unwrap();

                    // Go ahead and display all other lines
                    for line in lines.into_iter() {
                        eprintln!("{}", line);
                    }

                    // Get an answer from user input, or use a blank string as an answer
                    // if we fail to get input from the user
                    let answer = password_prompt(line).unwrap_or_default();

                    answers.push(answer);
                }
                answers
            }),
            on_verify: Box::new(move |kind, text| {
                trace!("[manager client] on_verify({kind}, {text})");
                match kind {
                    AuthVerifyKind::Host => {
                        eprintln!("{}", text);

                        match text_prompt("Enter [y/N]> ") {
                            Ok(answer) => {
                                trace!("Verify? Answer = '{answer}'");
                                matches!(answer.trim(), "y" | "Y" | "yes" | "YES")
                            }
                            Err(x) => {
                                error!("Failed verification: {x}");
                                false
                            }
                        }
                    }
                    x => {
                        error!("Unsupported verify kind: {x}");
                        false
                    }
                }
            }),
            on_info: Box::new(|text| {
                trace!("[manager client] on_info({text})");
                println!("{}", text);
            }),
            on_error: Box::new(|kind, text| {
                trace!("[manager client] on_error({kind}, {text})");
                eprintln!("{}: {}", kind, text);
            }),
        }
    }
}

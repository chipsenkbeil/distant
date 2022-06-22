use distant_net::{AuthChallengeFn, AuthErrorFn, AuthInfoFn, AuthVerifyFn, AuthVerifyKind};
use std::io;

/// Configuration to use when creating a new [`DistantManagerClient`](super::DistantManagerClient)
pub struct DistantManagerClientConfig {
    pub on_challenge: Box<AuthChallengeFn>,
    pub on_verify: Box<AuthVerifyFn>,
    pub on_info: Box<AuthInfoFn>,
    pub on_error: Box<AuthErrorFn>,
}

impl DistantManagerClientConfig {
    pub fn with_password_prompt<P>(prompt: P) -> Self
    where
        P: Fn(&str) -> io::Result<String> + Clone + Send + Sync + 'static,
    {
        Self {
            on_challenge: Box::new(move |questions, _extra| {
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
                    let answer = prompt(line).unwrap_or_default();

                    answers.push(answer);
                }
                answers
            }),
            on_verify: Box::new(move |kind, text| match kind {
                AuthVerifyKind::Host => {
                    eprintln!("{}", text);
                    match prompt("Enter [y/N]> ").as_deref() {
                        Ok("y" | "Y" | "yes" | "YES") => true,
                        _ => false,
                    }
                }
                _ => false,
            }),
            on_info: Box::new(|text| println!("{}", text)),
            on_error: Box::new(|kind, text| eprintln!("{}: {}", kind, text)),
        }
    }
}

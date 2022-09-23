use async_trait::async_trait;
use distant_net::auth::{AuthErrorKind, AuthHandler, AuthQuestion, AuthVerifyKind};
use log::*;
use std::{collections::HashMap, io};

/// Configuration to use when creating a new [`DistantManagerClient`](super::DistantManagerClient)
pub struct DistantManagerClientConfig {
    pub on_challenge:
        Box<dyn FnMut(Vec<AuthQuestion>, HashMap<String, String>) -> io::Result<Vec<String>>>,
    pub on_verify: Box<dyn FnMut(AuthVerifyKind, String) -> io::Result<bool>>,
    pub on_info: Box<dyn FnMut(String) -> io::Result<()>>,
    pub on_error: Box<dyn FnMut(AuthErrorKind, &str) -> io::Result<()>>,
}

#[async_trait]
impl AuthHandler for DistantManagerClientConfig {
    async fn on_challenge(
        &mut self,
        questions: Vec<AuthQuestion>,
        options: HashMap<String, String>,
    ) -> io::Result<Vec<String>> {
        (self.on_challenge)(questions, options)
    }

    async fn on_verify(&mut self, kind: AuthVerifyKind, text: String) -> io::Result<bool> {
        (self.on_verify)(kind, text)
    }

    async fn on_info(&mut self, text: String) -> io::Result<()> {
        (self.on_info)(text)
    }

    async fn on_error(&mut self, kind: AuthErrorKind, text: &str) -> io::Result<()> {
        (self.on_error)(kind, text)
    }
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
                Ok(answers)
            }),
            on_verify: Box::new(move |kind, text| {
                trace!("[manager client] on_verify({kind}, {text})");
                match kind {
                    AuthVerifyKind::Host => {
                        eprintln!("{}", text);

                        let answer = text_prompt("Enter [y/N]> ")?;
                        trace!("Verify? Answer = '{answer}'");
                        Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "YES"))
                    }
                    x => {
                        error!("Unsupported verify kind: {x}");
                        Ok(false)
                    }
                }
            }),
            on_info: Box::new(|text| {
                trace!("[manager client] on_info({text})");
                println!("{}", text);
                Ok(())
            }),
            on_error: Box::new(|kind, text| {
                trace!("[manager client] on_error({kind}, {text})");
                eprintln!("{}: {}", kind, text);
                Ok(())
            }),
        }
    }
}

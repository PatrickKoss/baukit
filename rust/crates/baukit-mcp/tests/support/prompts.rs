use baukit_mcp::{
    CapabilityError, Principal, Prompt, PromptArgument, PromptFuture, PromptMessage, PromptResult,
    PromptService, Role, ScopedPrompt,
};
use serde_json::Value;
use std::sync::Mutex;

#[derive(Default)]
pub struct Recommendations(pub Mutex<Vec<(String, Value)>>);

impl PromptService for Recommendations {
    fn list(&self) -> Vec<ScopedPrompt> {
        vec![
            ScopedPrompt {
                prompt: Prompt::new(
                    "next-steps",
                    Some("Recommend practice"),
                    Some(vec![
                        PromptArgument::new("language")
                            .with_description("Output language")
                            .with_required(true),
                    ]),
                ),
                required_scopes: vec!["learning:read".into()],
            },
            ScopedPrompt {
                prompt: Prompt::new("private-steps", Some("Private recommendation"), None),
                required_scopes: vec!["learning:read".into(), "private:read".into()],
            },
            ScopedPrompt {
                prompt: Prompt::new(
                    "recommend-next-steps",
                    Some("Recommend practice without arguments"),
                    None,
                ),
                required_scopes: vec!["learning:read".into()],
            },
        ]
    }
    fn get<'a>(
        &'a self,
        principal: &'a Principal,
        _name: &'a str,
        arguments: Value,
    ) -> PromptFuture<'a> {
        Box::pin(async move {
            self.0
                .lock()
                .expect("prompts")
                .push((principal.subject().into(), arguments.clone()));
            if arguments["language"] == "unavailable" {
                return Err(CapabilityError::Internal {
                    message: "Recommendations unavailable".into(),
                    data: None,
                });
            }
            Ok(PromptResult::new(vec![PromptMessage::new_text(
                Role::User,
                format!(
                    "Recommend practice for {} in {}",
                    principal.subject(),
                    arguments["language"]
                ),
            )])
            .with_description("Practice recommendation"))
        })
    }
}

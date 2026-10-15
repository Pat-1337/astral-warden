#[derive(Clone, PartialEq, Eq)]
pub enum Scope {
    AllUsers,
    User(String),
}

impl Scope {
    pub fn parse(text: &str) -> Result<Self, String> {
        let sid = text.strip_prefix("S-1-").is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .split('-')
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        });

        match text {
            "all" => Ok(Self::AllUsers),
            _ if sid => Ok(Self::User(text.to_owned())),
            _ => Err(format!("--scope must be \"all\" or a user SID, not {text:?}")),
        }
    }

    pub fn arg(&self) -> &str {
        match self {
            Self::AllUsers => "all",
            Self::User(sid) => sid,
        }
    }
}

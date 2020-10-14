use super::{try_link_name, Args};
use crate::Context;

use std::str::FromStr;

pub struct RankArgs {
    pub name: Option<String>,
    pub country: Option<String>,
    pub rank: usize,
}

impl RankArgs {
    pub fn new(ctx: &Context, args: Args) -> Result<Self, &'static str> {
        let mut args = args.take_all();
        let (country, rank) = if let Some(arg) = args.next_back() {
            if let Ok(num) = usize::from_str(arg) {
                (None, num)
            } else if arg.len() < 3 {
                return Err("Could not parse rank. Provide it either as positive \
                    number or as country acronym followed by a positive \
                    number e.g. `be10`.");
            } else {
                let (country, num) = arg.split_at(2);
                match (usize::from_str(num), country.chars().all(|c| c.is_ascii_alphabetic())) {
                    (Ok(num), true) => (Some(country.to_uppercase()), num),
                    (Err(_), _) => {
                        return Err("Could not parse rank. Provide it either as positive \
                                    number or as country acronym followed by a positive \
                                    number e.g. `be10`.")
                    }
                    (_, false) => {
                        return Err(
                            "Could not parse country. Be sure to specify it with two letters, e.g. `be10`.",
                        )
                    }
                }
            }
        } else {
            return Err(
                "No rank argument found. Provide it either as positive number or \
                 as country acronym followed by a positive number e.g. `be10`.",
            );
        };
        let name = try_link_name(ctx, args.next());
        Ok(Self {
            name,
            country,
            rank,
        })
    }
}

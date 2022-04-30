use chrono::{DateTime, Utc};
use command_macros::BasePagination;
use twilight_model::channel::Message;

use crate::{embeds::CommandCounterEmbed, BotResult, Context};

use super::{Pages, Pagination};

#[derive(BasePagination)]
#[pagination(no_multi)]
pub struct CommandCountPagination {
    msg: Message,
    pages: Pages,
    booted_up: DateTime<Utc>,
    cmd_counts: Vec<(String, u32)>,
}

impl CommandCountPagination {
    pub fn new(ctx: &Context, msg: Message, cmd_counts: Vec<(String, u32)>) -> Self {
        let booted_up = ctx.stats.start_time;
        Self {
            msg,
            pages: Pages::new(15, cmd_counts.len()),
            cmd_counts,
            booted_up,
        }
    }
}

#[async_trait]
impl Pagination for CommandCountPagination {
    type PageData = CommandCounterEmbed;

    async fn build_page(&mut self) -> BotResult<Self::PageData> {
        let sub_list: Vec<(&String, u32)> = self
            .cmd_counts
            .iter()
            .skip(self.pages.index)
            .take(self.pages.per_page)
            .map(|(name, amount)| (name, *amount))
            .collect();

        Ok(CommandCounterEmbed::new(
            sub_list,
            &self.booted_up,
            self.pages.index + 1,
            (self.page(), self.pages.total_pages),
        ))
    }
}

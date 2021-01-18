use super::{Pages, Pagination};

use crate::{embeds::ScoresEmbed, BotResult};

use async_trait::async_trait;
use rosu::model::{Beatmap, Score, User};
use twilight_model::channel::Message;

pub struct ScoresPagination {
    msg: Message,
    pages: Pages,
    user: User,
    map: Beatmap,
    scores: Vec<Score>,
}

impl ScoresPagination {
    pub fn new(msg: Message, user: User, map: Beatmap, scores: Vec<Score>) -> Self {
        Self {
            msg,
            pages: Pages::new(10, scores.len()),
            user,
            map,
            scores,
        }
    }
}

#[async_trait]
impl Pagination for ScoresPagination {
    type PageData = ScoresEmbed;

    fn msg(&self) -> &Message {
        &self.msg
    }

    fn pages(&self) -> Pages {
        self.pages
    }

    fn pages_mut(&mut self) -> &mut Pages {
        &mut self.pages
    }

    fn single_step(&self) -> usize {
        self.pages.per_page
    }

    async fn build_page(&mut self) -> BotResult<Self::PageData> {
        let scores = self
            .scores
            .iter()
            .skip(self.pages.index)
            .take(self.pages.per_page);

        Ok(ScoresEmbed::new(&self.user, &self.map, scores, self.pages.index).await)
    }
}

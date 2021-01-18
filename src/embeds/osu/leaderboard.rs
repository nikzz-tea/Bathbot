use crate::{
    custom_client::ScraperScore,
    embeds::{Author, EmbedData, Footer},
    pp::{Calculations, PPCalculator},
    unwind_error,
    util::{
        constants::{AVATAR_URL, MAP_THUMB_URL, OSU_BASE},
        datetime::how_long_ago,
        numbers::with_comma_u64,
        ScoreExt,
    },
    BotResult,
};

use cow_utils::CowUtils;
use rosu::model::{Beatmap, GameMode};
use std::{borrow::Cow, collections::HashMap, fmt::Write};
use twilight_embed_builder::image_source::ImageSource;

pub struct LeaderboardEmbed {
    description: String,
    thumbnail: ImageSource,
    author: Author,
    footer: Footer,
}

impl LeaderboardEmbed {
    pub async fn new<'i, S>(
        init_name: Option<&str>,
        map: &Beatmap,
        scores: Option<S>,
        author_icon: &Option<String>,
        idx: usize,
    ) -> BotResult<Self>
    where
        S: Iterator<Item = &'i ScraperScore>,
    {
        let mut author_text = String::with_capacity(32);
        if map.mode == GameMode::MNA {
            let _ = write!(author_text, "[{}K] ", map.diff_cs as u32);
        }
        let _ = write!(author_text, "{} [{:.2}★]", map, map.stars);
        let description = if let Some(scores) = scores {
            let mut mod_map = HashMap::new();
            let mut description = String::with_capacity(256);
            let author_name = init_name.map_or_else(|| Cow::Borrowed(""), |n| n.cow_to_lowercase());
            for (i, score) in scores.enumerate() {
                let found_author = author_name == score.username.cow_to_lowercase();
                let mut username = String::with_capacity(32);
                if found_author {
                    username.push_str("__");
                }
                let _ = write!(
                    username,
                    "[{name}]({base}users/{id})",
                    name = score.username,
                    base = OSU_BASE,
                    id = score.user_id
                );
                if found_author {
                    username.push_str("__");
                }
                let _ = writeln!(
                    description,
                    "**{idx}.** {grade} **{name}**: {score} [ {combo} ]{mods}\n\
                    - {pp} ~ {acc:.2}% ~ {ago}",
                    idx = idx + i + 1,
                    grade = score.grade_emote(map.mode),
                    name = username,
                    score = with_comma_u64(score.score as u64),
                    combo = get_combo(&score, &map),
                    mods = if score.enabled_mods.is_empty() {
                        String::new()
                    } else {
                        format!(" **+{}**", score.enabled_mods)
                    },
                    pp = get_pp(&mut mod_map, &score, &map).await,
                    acc = score.accuracy,
                    ago = how_long_ago(&score.date),
                );
            }

            description
        } else {
            "No scores found".to_string()
        };
        let mut author = Author::new(author_text).url(format!("{}b/{}", OSU_BASE, map.beatmap_id));
        if let Some(ref author_icon) = author_icon {
            author = author.icon_url(author_icon.to_owned());
        }
        let footer = Footer::new(format!("{:?} map by {}", map.approval_status, map.creator))
            .icon_url(format!("{}{}", AVATAR_URL, map.creator_id));

        Ok(Self {
            author,
            description,
            footer,
            thumbnail: ImageSource::url(format!("{}{}l.jpg", MAP_THUMB_URL, map.beatmapset_id))
                .unwrap(),
        })
    }
}

impl EmbedData for LeaderboardEmbed {
    fn description(&self) -> Option<&str> {
        Some(&self.description)
    }
    fn author(&self) -> Option<&Author> {
        Some(&self.author)
    }
    fn footer(&self) -> Option<&Footer> {
        Some(&self.footer)
    }
    fn thumbnail(&self) -> Option<&ImageSource> {
        Some(&self.thumbnail)
    }
}

// TODO: Optimize through rosu-pp
async fn get_pp(mod_map: &mut HashMap<u32, f32>, score: &ScraperScore, map: &Beatmap) -> String {
    let mut calculator = PPCalculator::new().score(score).map(map);
    let mut calculations = Calculations::PP;
    let bits = score.enabled_mods.bits();
    if !mod_map.contains_key(&bits) {
        calculations |= Calculations::MAX_PP;
    }
    let (pp, max_pp) = match calculator.calculate(calculations).await {
        Ok(_) => {
            let pp = calculator.pp().unwrap();
            let max_pp = match calculator.max_pp() {
                Some(pp) => {
                    mod_map.insert(bits, pp);
                    pp
                }
                None => *mod_map.get(&bits).unwrap(),
            };
            (pp, max_pp)
        }
        Err(why) => {
            unwind_error!(warn, why, "Error while calculating pp, defaulting to 0: {}");
            (0.0, 0.0)
        }
    };
    format!("**{:.2}**/{:.2}PP", pp, max_pp)
}

fn get_combo(score: &ScraperScore, map: &Beatmap) -> String {
    let mut combo = format!("**{}x**/", score.max_combo);
    let _ = if let Some(amount) = map.max_combo {
        write!(combo, "{}x", amount)
    } else {
        write!(
            combo,
            " {} miss{}",
            score.count_miss,
            if score.count_miss != 1 { "es" } else { "" }
        )
    };
    combo
}

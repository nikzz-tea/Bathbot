use std::{sync::Arc, time::Duration};

use bathbot_macros::SlashCommand;
use bathbot_model::{PersonalBestIndex, ScoreSlim, embed_builder::ScoreEmbedSettings};
use bathbot_psql::model::configs::ScoreData;
use bathbot_util::{
    Authored, CowUtils, MessageOrigin,
    constants::GENERAL_ISSUE,
    query::{FilterCriteria, Searchable, TopCriteria},
};
use eyre::{Report, Result};
use rosu_pp::model::beatmap::BeatmapAttributes;
use rosu_v2::{
    model::{GameMode, Grade},
    prelude::{GameModIntermode, GameMods, RankStatus, Score, ScoreStatistics},
};
use time::OffsetDateTime;
use twilight_interactions::command::{CommandModel, CreateCommand};
use twilight_model::id::{
    Id,
    marker::{GuildMarker, UserMarker},
};

use crate::{
    active::{ActiveMessages, impls::ScoreEmbedBuilderActive},
    core::Context,
    manager::{MapError, OsuMap, PpManager, redis::osu::UserArgsSlim},
    util::{InteractionCommandExt, interaction::InteractionCommand, osu::IfFc},
};

const USER_ID: u32 = 2;
const MAP_ID: u32 = 197337;
const MODE: GameMode = GameMode::Osu;
const MISS_ANALYZER_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(CommandModel, CreateCommand, SlashCommand)]
#[command(name = "builder", desc = "Build your own score embed format")]
#[flags(EPHEMERAL)]
pub enum ScoreEmbedBuilder {
    #[command(name = "edit")]
    Edit(ScoreEmbedBuilderEdit),
    #[command(name = "copy")]
    Copy(ScoreEmbedBuilderCopy),
    #[command(name = "default")]
    Default(ScoreEmbedBuilderDefault),
}

#[derive(CommandModel, CreateCommand)]
#[command(name = "edit", desc = "Edit your score embed format")]
pub struct ScoreEmbedBuilderEdit;

#[derive(CommandModel, CreateCommand)]
#[command(
    name = "copy",
    desc = "Use someone else's score embed format as your own"
)]
pub struct ScoreEmbedBuilderCopy {
    #[command(desc = "Specify a user to copy the score embed format from")]
    user: Id<UserMarker>,
}

#[derive(CommandModel, CreateCommand)]
#[command(
    name = "default",
    desc = "Reset your score embed format to the default"
)]
pub struct ScoreEmbedBuilderDefault;

pub async fn slash_scoreembedbuilder(mut command: InteractionCommand) -> Result<()> {
    match ScoreEmbedBuilder::from_interaction(command.input_data())? {
        ScoreEmbedBuilder::Edit(_) => edit(&mut command).await,
        ScoreEmbedBuilder::Copy(args) => copy(&mut command, args).await,
        ScoreEmbedBuilder::Default(_) => default(&mut command).await,
    }
}

async fn edit(command: &mut InteractionCommand) -> Result<()> {
    let config = match Context::user_config().with_osu_id(command.user_id()?).await {
        Ok(config) => config,
        Err(err) => {
            let _ = command.error(GENERAL_ISSUE).await;

            return Err(err.wrap_err("Failed to get user config"));
        }
    };

    let score_data = match config.score_data {
        Some(score_data) => score_data,
        None => match command.guild_id() {
            Some(guild_id) => Context::guild_config()
                .peek(guild_id, |config| config.score_data)
                .await
                .unwrap_or_default(),
            None => Default::default(),
        },
    };

    let settings = config.score_embed.unwrap_or_default();

    exec(command, settings, score_data).await
}

async fn copy(command: &mut InteractionCommand, args: ScoreEmbedBuilderCopy) -> Result<()> {
    let author = command.user_id()?;

    let config_fut1 = Context::user_config().with_osu_id(author);
    let config_fut2 = Context::user_config().with_osu_id(args.user);

    let (config1, config2) = match tokio::try_join!(config_fut1, config_fut2) {
        Ok(tuple) => tuple,
        Err(err) => {
            let _ = command.error(GENERAL_ISSUE).await;

            return Err(err.wrap_err("Failed to get user config"));
        }
    };

    let score_data = match config1.score_data {
        Some(score_data) => score_data,
        None => match command.guild_id() {
            Some(guild_id) => Context::guild_config()
                .peek(guild_id, |config| config.score_data)
                .await
                .unwrap_or_default(),
            None => Default::default(),
        },
    };

    let settings = config2.score_embed.unwrap_or_default();

    let store_fut = Context::user_config().store_score_embed_settings(author, &settings);

    if let Err(err) = store_fut.await {
        warn!(?err);
    }

    exec(command, settings, score_data).await
}

async fn default(command: &mut InteractionCommand) -> Result<()> {
    let author = command.user_id()?;

    let config = match Context::user_config().with_osu_id(author).await {
        Ok(config) => config,
        Err(err) => {
            let _ = command.error(GENERAL_ISSUE).await;

            return Err(err.wrap_err("Failed to get user config"));
        }
    };

    let score_data = match config.score_data {
        Some(score_data) => score_data,
        None => match command.guild_id() {
            Some(guild_id) => Context::guild_config()
                .peek(guild_id, |config| config.score_data)
                .await
                .unwrap_or_default(),
            None => Default::default(),
        },
    };

    let settings = ScoreEmbedSettings::default();

    let store_fut = Context::user_config().store_score_embed_settings(author, &settings);

    if let Err(err) = store_fut.await {
        warn!(?err);
    }

    exec(command, settings, score_data).await
}

async fn exec(
    command: &mut InteractionCommand,
    settings: ScoreEmbedSettings,
    score_data: ScoreData,
) -> Result<()> {
    let msg_owner = command.user_id()?;
    let legacy_scores = score_data.is_legacy();

    let user_fut = Context::redis().osu_user_from_args(UserArgsSlim::user_id(USER_ID));

    let score_fut =
        Context::osu_scores().user_on_map_single(USER_ID, MAP_ID, MODE, None, legacy_scores);

    let map_fut = Context::osu_map().map(MAP_ID, None);

    let (user, score, map) = match tokio::join!(user_fut, score_fut, map_fut) {
        (Ok(user), Ok(score), Ok(map)) => (user, score.score, map),
        (user_res, score_res, map_res) => {
            let _ = command.error(GENERAL_ISSUE).await;

            let (err, wrap) = if let Err(err) = user_res {
                (Report::new(err), "Failed to get user for builder")
            } else if let Err(err) = score_res {
                (Report::new(err), "Failed to get score for builder")
            } else if let Err(err) = map_res {
                (Report::new(err), "Failed to get map for builder")
            } else {
                unreachable!()
            };

            return Err(err.wrap_err(wrap));
        }
    };

    let mut data = ScoreEmbedDataWrap::new_custom(score, map, 71, Some(7)).await;

    // Adjusting hitresults to better showcase the "Ratio" value
    if let ScoreEmbedDataStatus::Full(ref mut data) = data.inner {
        data.score.statistics.perfect = 480;
    }

    let active_msg = ScoreEmbedBuilderActive::new(&user, data, settings, score_data, msg_owner);

    ActiveMessages::builder(active_msg)
        .start_by_update(true)
        .begin(command)
        .await
}

pub struct ScoreEmbedDataWrap {
    inner: ScoreEmbedDataStatus,
}

impl ScoreEmbedDataWrap {
    /// Create a [`ScoreEmbedDataWrap`] with a [`Score`] and only some
    /// metadata.
    pub fn new_raw(
        score: Score,
        legacy_scores: bool,
        with_render: bool,
        miss_analyzer: MissAnalyzerCheck,
        top100: Option<Arc<[Score]>>,
        #[cfg(feature = "twitch")] twitch_data: Option<Arc<TwitchData>>,
        origin: MessageOrigin,
    ) -> Self {
        Self {
            inner: ScoreEmbedDataStatus::Raw(Some(ScoreEmbedDataRaw::new(
                score,
                legacy_scores,
                with_render,
                miss_analyzer,
                top100,
                #[cfg(feature = "twitch")]
                twitch_data,
                origin,
            ))),
        }
    }

    /// Create a [`ScoreEmbedDataWrap`] with a [`Score`], an [`OsuMap`], and
    /// only some metadata.
    pub async fn new_half(
        score: Score,
        map: OsuMap,
        pb_idx: Option<ScoreEmbedDataPersonalBest>,
        legacy_scores: bool,
        with_render: bool,
        miss_analyzer: MissAnalyzerCheck,
    ) -> Self {
        Self {
            inner: ScoreEmbedDataStatus::Half(Some(
                ScoreEmbedDataHalf::new(
                    score,
                    map,
                    pb_idx,
                    legacy_scores,
                    with_render,
                    miss_analyzer,
                )
                .await,
            )),
        }
    }

    pub async fn new_custom(
        score: Score,
        map: OsuMap,
        pb_idx: usize,
        global_idx: Option<usize>,
    ) -> Self {
        let PpAttrs {
            calc,
            stars,
            max_combo,
            max_pp,
        } = PpAttrs::new(
            &map,
            score.mode,
            &score.mods,
            score.grade,
            score.set_on_lazer,
            score.pp,
        )
        .await;

        let pp = match score.pp {
            Some(pp) => pp,
            None => match calc.score(&score).performance().await {
                Some(attrs) => attrs.pp() as f32,
                None => 0.0,
            },
        };

        let score = ScoreSlim::new(score, pp);

        let if_fc_pp = IfFc::new(&score, &map).await.map(|if_fc| if_fc.pp);

        Self {
            inner: ScoreEmbedDataStatus::Full(ScoreEmbedData {
                score,
                map,
                stars,
                max_combo,
                max_pp,
                replay_score_id: None,
                miss_analyzer: None,
                pb_idx: Some(ScoreEmbedDataPersonalBest::from_index(pb_idx)),
                global_idx,
                if_fc_pp,
                #[cfg(feature = "twitch")]
                twitch: None,
            }),
        }
    }

    /// Returns the inner [`ScoreEmbedData`].
    ///
    /// If the data has not yet been calculated, it will do so first.
    pub async fn get_mut(&mut self) -> Result<&mut ScoreEmbedData> {
        let data = match self.inner {
            ScoreEmbedDataStatus::Raw(ref mut raw) => {
                let raw = raw
                    .take()
                    .ok_or_else(|| eyre!("Raw data was already taken"))?;

                self.inner = ScoreEmbedDataStatus::Empty;

                raw.into_full().await?
            }
            ScoreEmbedDataStatus::Half(ref mut half) => {
                let half = half
                    .take()
                    .ok_or_else(|| eyre!("Half data was already taken"))?;

                self.inner = ScoreEmbedDataStatus::Empty;

                half.into_full().await
            }
            ScoreEmbedDataStatus::Full(ref mut data) => return Ok(data),
            ScoreEmbedDataStatus::Empty => bail!("Empty data"),
        };

        self.inner = ScoreEmbedDataStatus::Full(data);

        let ScoreEmbedDataStatus::Full(ref mut data) = self.inner else {
            unreachable!()
        };

        Ok(data)
    }

    /// Returns the inner [`ScoreEmbedData`].
    ///
    /// If the data has not yet been calculated, returns `None`.
    pub fn try_get(&self) -> Option<&ScoreEmbedData> {
        self.inner.try_get()
    }

    pub fn try_get_half(&self) -> Option<&ScoreEmbedDataHalf> {
        if let ScoreEmbedDataStatus::Half(ref half) = self.inner {
            half.as_ref()
        } else {
            None
        }
    }

    #[track_caller]
    pub fn get_half(&self) -> &ScoreEmbedDataHalf {
        self.try_get_half().unwrap()
    }
}

impl From<ScoreEmbedDataHalf> for ScoreEmbedDataWrap {
    fn from(data: ScoreEmbedDataHalf) -> Self {
        Self {
            inner: ScoreEmbedDataStatus::Half(Some(data)),
        }
    }
}

enum ScoreEmbedDataStatus {
    Raw(Option<ScoreEmbedDataRaw>),
    Half(Option<ScoreEmbedDataHalf>),
    Full(ScoreEmbedData),
    Empty,
}

impl ScoreEmbedDataStatus {
    fn try_get(&self) -> Option<&ScoreEmbedData> {
        match self {
            Self::Full(data) => Some(data),
            Self::Raw(_) | Self::Half(_) | Self::Empty => None,
        }
    }
}

pub struct ScoreEmbedDataHalf {
    pub user_id: u32,
    pub score: ScoreSlim,
    pub map: OsuMap,
    pub stars: f32,
    pub max_combo: u32,
    pub max_pp: f32,
    pub pb_idx: Option<ScoreEmbedDataPersonalBest>,
    pub legacy_scores: bool,
    pub with_render: bool,
    pub has_replay: bool,
    pub miss_analyzer_check: MissAnalyzerCheck,
    pub original_idx: Option<usize>,
}

impl ScoreEmbedDataHalf {
    pub async fn new(
        score: Score,
        map: OsuMap,
        pb_idx: Option<ScoreEmbedDataPersonalBest>,
        legacy_scores: bool,
        with_render: bool,
        miss_analyzer_check: MissAnalyzerCheck,
    ) -> Self {
        let user_id = score.user_id;

        let PpAttrs {
            calc,
            stars,
            max_combo,
            max_pp,
        } = PpAttrs::new(
            &map,
            score.mode,
            &score.mods,
            score.grade,
            score.set_on_lazer,
            score.pp,
        )
        .await;

        let pp = match score.pp {
            Some(pp) => pp,
            None => match calc.score(&score).performance().await {
                Some(attrs) => attrs.pp() as f32,
                None => 0.0,
            },
        };

        let has_replay = score.has_replay;
        let score = ScoreSlim::new(score, pp);

        Self {
            user_id,
            score,
            map,
            stars,
            max_combo,
            max_pp,
            pb_idx,
            legacy_scores,
            with_render,
            has_replay,
            miss_analyzer_check,
            original_idx: None,
        }
    }

    async fn into_full(self) -> ScoreEmbedData {
        let global_idx_fut = async {
            if !matches!(
                self.map.status(),
                RankStatus::Ranked
                    | RankStatus::Loved
                    | RankStatus::Qualified
                    | RankStatus::Approved
            ) || self.score.grade == Grade::F
            {
                return None;
            }

            let map_lb_fut = Context::osu_scores().map_leaderboard(
                self.map.map_id(),
                self.score.mode,
                None,
                50,
                self.legacy_scores,
            );

            let scores = match map_lb_fut.await {
                Ok(scores) => scores,
                Err(err) => {
                    warn!(?err, "Failed to get global scores");

                    return None;
                }
            };

            scores
                .iter()
                .position(|s| s.user_id == self.user_id && self.score.is_eq(s))
                .map(|idx| idx + 1)
        };

        let miss_analyzer_fut = async {
            let guild_id = self
                .miss_analyzer_check
                .guild_id
                .filter(|_| !self.score.is_legacy)?;

            let score_id = self.score.score_id;

            debug!(score_id, "Sending score id to miss analyzer");

            let miss_analyzer_fut =
                Context::client().miss_analyzer_score_request(guild_id.get(), score_id);

            match tokio::time::timeout(MISS_ANALYZER_TIMEOUT, miss_analyzer_fut).await {
                Ok(Ok(wants_button)) => wants_button.then_some(MissAnalyzerData { score_id }),
                Ok(Err(err)) => {
                    warn!(?err, "Failed to send score id to miss analyzer");

                    None
                }
                Err(_) => {
                    warn!("Miss analyzer request timed out");

                    None
                }
            }
        };

        let if_fc_fut = IfFc::new(&self.score, &self.map);

        let (global_idx, if_fc, miss_analyzer) =
            tokio::join!(global_idx_fut, if_fc_fut, miss_analyzer_fut);

        let if_fc_pp = if_fc.map(|if_fc| if_fc.pp);

        let replay_score_id = (self.with_render && self.has_replay && !self.score.is_legacy)
            .then_some(self.score.score_id);

        ScoreEmbedData {
            score: self.score,
            map: self.map,
            stars: self.stars,
            max_combo: self.max_combo,
            max_pp: self.max_pp,
            replay_score_id,
            miss_analyzer,
            pb_idx: self.pb_idx,
            global_idx,
            if_fc_pp,
            #[cfg(feature = "twitch")]
            twitch: None,
        }
    }

    fn map_attrs(&self) -> BeatmapAttributes {
        self.map.attributes().mods(self.score.mods.clone()).build()
    }

    pub fn ar(&self) -> f64 {
        self.map_attrs().ar
    }

    pub fn cs(&self) -> f64 {
        self.map_attrs().cs
    }

    pub fn hp(&self) -> f64 {
        self.map_attrs().hp
    }

    pub fn od(&self) -> f64 {
        self.map_attrs().od
    }
}

pub struct ScoreEmbedData {
    pub score: ScoreSlim,
    pub map: OsuMap,
    pub stars: f32,
    pub max_combo: u32,
    pub max_pp: f32,
    pub replay_score_id: Option<u64>,
    pub miss_analyzer: Option<MissAnalyzerData>,
    pub pb_idx: Option<ScoreEmbedDataPersonalBest>,
    pub global_idx: Option<usize>,
    pub if_fc_pp: Option<f32>,
    #[cfg(feature = "twitch")]
    pub twitch: Option<Arc<TwitchData>>,
}

#[cfg(feature = "twitch")]
pub enum TwitchData {
    Vod {
        vod: bathbot_cache::model::CachedArchive<bathbot_model::ArchivedTwitchVideo>,
        stream: bathbot_cache::model::CachedArchive<bathbot_model::ArchivedTwitchStream>,
    },
    Stream(bathbot_cache::model::CachedArchive<bathbot_model::ArchivedTwitchStream>),
}

#[cfg(feature = "twitch")]
const _: () = {
    use std::fmt::Write;

    use rkyv::rancor::{Panic, ResultExt};

    impl TwitchData {
        pub fn append_to_description(
            &self,
            score: &ScoreSlim,
            map: &OsuMap,
            description: &mut String,
        ) {
            match self {
                TwitchData::Vod { vod, stream } => {
                    let score_start = Self::score_started_at(score, map);
                    let vod_start = vod.created_at.try_deserialize::<Panic>().always_ok();
                    let vod_end = vod.ended_at();

                    if vod_start < score_start && score_start < vod_end {
                        Self::append_vod_to_description(vod, score_start, description);
                    } else {
                        Self::append_stream_to_description(stream.login.as_str(), description);
                    }
                }
                TwitchData::Stream(stream) => {
                    Self::append_stream_to_description(stream.login.as_str(), description)
                }
            }
        }

        fn score_started_at(score: &ScoreSlim, map: &OsuMap) -> OffsetDateTime {
            let mut map_len = map.seconds_drain() as f64;

            if score.grade == Grade::F {
                // Adjust map length with passed objects in case of fail
                let passed = score.total_hits();

                if map.mode() == GameMode::Catch {
                    // Amount objects in .osu file != amount of catch hitobjects
                    map_len += 2.0;
                } else if let Some(obj) = passed
                    .checked_sub(1)
                    .and_then(|i| map.pp_map.hit_objects.get(i as usize))
                {
                    map_len = obj.start_time / 1000.0;
                } else {
                    let total = map.n_objects();
                    map_len *= passed as f64 / total as f64;

                    map_len += 2.0;
                }
            } else {
                map_len += map.pp_map.total_break_time() / 1000.0;
            }

            if let Some(clock_rate) = score.mods.clock_rate() {
                map_len /= clock_rate;
            }

            score.ended_at - std::time::Duration::from_secs(map_len as u64 + 3)
        }

        fn append_vod_to_description(
            vod: &bathbot_model::ArchivedTwitchVideo,
            score_start: OffsetDateTime,
            description: &mut String,
        ) {
            let _ = write!(
                description,
                "{emote} [Liveplay on twitch]({url}",
                emote = crate::util::Emote::Twitch,
                url = vod.url
            );

            description.push_str("?t=");
            let mut offset = (score_start - vod.created_at.try_deserialize::<Panic>().always_ok())
                .whole_seconds();

            if offset >= 3600 {
                let _ = write!(description, "{}h", offset / 3600);
                offset %= 3600;
            }

            if offset >= 60 {
                let _ = write!(description, "{}m", offset / 60);
                offset %= 60;
            }

            if offset > 0 {
                let _ = write!(description, "{offset}s");
            }

            description.push(')');
        }

        fn append_stream_to_description(login: &str, description: &mut String) {
            let _ = write!(
                description,
                "{emote} [Streaming on twitch]({base}{login})",
                emote = crate::util::Emote::Twitch,
                base = bathbot_util::constants::TWITCH_BASE
            );
        }
    }
};

pub struct ScoreEmbedDataRaw {
    pub user_id: u32,
    pub map_id: u32,
    pub checksum: Option<String>,
    pub set_on_lazer: bool,
    pub legacy_scores: bool,
    pub with_render: bool,
    pub miss_analyzer_check: MissAnalyzerCheck,
    pub top100: Option<Arc<[Score]>>,
    #[cfg(feature = "twitch")]
    pub twitch: Option<Arc<TwitchData>>,
    pub origin: MessageOrigin,
    pub accuracy: f32,
    pub ended_at: OffsetDateTime,
    pub grade: Grade,
    pub max_combo: u32,
    pub mode: GameMode,
    pub mods: GameMods,
    pub pp: Option<f32>,
    pub score: u32,
    pub classic_score: u64,
    pub score_id: u64,
    /// See [`ScoreSlim::is_legacy`]
    pub is_legacy: bool,
    pub statistics: ScoreStatistics,
    pub has_replay: bool,
}

impl ScoreEmbedDataRaw {
    fn new(
        score: Score,
        legacy_scores: bool,
        with_render: bool,
        miss_analyzer_check: MissAnalyzerCheck,
        top100: Option<Arc<[Score]>>,
        #[cfg(feature = "twitch")] twitch_data: Option<Arc<TwitchData>>,
        origin: MessageOrigin,
    ) -> Self {
        Self {
            user_id: score.user_id,
            map_id: score.map_id,
            checksum: score.map.and_then(|map| map.checksum),
            legacy_scores,
            with_render,
            miss_analyzer_check,
            top100,
            #[cfg(feature = "twitch")]
            twitch: twitch_data,
            origin,
            accuracy: score.accuracy,
            ended_at: score.ended_at,
            grade: if score.passed { score.grade } else { Grade::F },
            max_combo: score.max_combo,
            mode: score.mode,
            mods: score.mods,
            pp: score.pp,
            score: score.score,
            classic_score: score.classic_score,
            score_id: score.id,
            is_legacy: score.legacy_score_id == Some(score.id),
            statistics: score.statistics,
            has_replay: score.replay,
            set_on_lazer: score.set_on_lazer,
        }
    }

    async fn into_full(self) -> Result<ScoreEmbedData> {
        let map_id = self.map_id;
        let checksum = self.checksum.as_deref();

        let map_fut = Context::osu_map().map(map_id, checksum);

        let map = match map_fut.await {
            Ok(map) => map.convert(self.mode),
            Err(MapError::NotFound) => bail!("Beatmap with id {map_id} was not found"),
            Err(MapError::Report(err)) => return Err(err),
        };

        let PpAttrs {
            calc,
            stars,
            max_combo,
            max_pp,
        } = PpAttrs::new(
            &map,
            self.mode,
            &self.mods,
            self.grade,
            self.set_on_lazer,
            self.pp,
        )
        .await;

        let pp = match self.pp {
            Some(pp) => pp,
            None => match calc.score(&self).performance().await {
                Some(attrs) => attrs.pp() as f32,
                None => 0.0,
            },
        };

        let score = ScoreSlim {
            accuracy: self.accuracy,
            ended_at: self.ended_at,
            grade: self.grade,
            max_combo: self.max_combo,
            mode: self.mode,
            mods: self.mods,
            pp,
            score: self.score,
            classic_score: self.classic_score,
            score_id: self.score_id,
            is_legacy: self.is_legacy,
            statistics: self.statistics,
            set_on_lazer: self.set_on_lazer,
        };

        let global_idx_fut = async {
            if !matches!(
                map.status(),
                RankStatus::Ranked
                    | RankStatus::Loved
                    | RankStatus::Qualified
                    | RankStatus::Approved
            ) || score.grade == Grade::F
            {
                return None;
            }

            let map_lb_fut = Context::osu_scores().map_leaderboard(
                map_id,
                score.mode,
                None,
                50,
                self.legacy_scores,
            );

            let scores = match map_lb_fut.await {
                Ok(scores) => scores,
                Err(err) => {
                    warn!(?err, "Failed to get global scores");

                    return None;
                }
            };

            scores
                .iter()
                .position(|s| s.user_id == self.user_id && score.is_eq(s))
                .map(|idx| idx + 1)
        };

        let miss_analyzer_fut = async {
            let guild_id = self
                .miss_analyzer_check
                .guild_id
                .filter(|_| self.has_replay && !self.is_legacy)?;

            let score_id = self.score_id;

            debug!(score_id, "Sending score id to miss analyzer");

            let miss_analyzer_fut =
                Context::client().miss_analyzer_score_request(guild_id.get(), score_id);

            match tokio::time::timeout(MISS_ANALYZER_TIMEOUT, miss_analyzer_fut).await {
                Ok(Ok(wants_button)) => wants_button.then_some(MissAnalyzerData { score_id }),
                Ok(Err(err)) => {
                    warn!(?err, "Failed to send score id to miss analyzer");

                    None
                }
                Err(_) => {
                    warn!("Miss analyzer request timed out");

                    None
                }
            }
        };

        let if_fc_fut = IfFc::new(&score, &map);

        let (global_idx, if_fc, miss_analyzer) =
            tokio::join!(global_idx_fut, if_fc_fut, miss_analyzer_fut);

        let if_fc_pp = if_fc.map(|if_fc| if_fc.pp);

        let replay_score_id =
            (self.with_render && self.has_replay && !self.is_legacy).then_some(score.score_id);

        let pb_idx = self
            .top100
            .as_deref()
            .map(|top100| PersonalBestIndex::new(&score, map_id, map.status(), top100))
            .and_then(|pb_idx| ScoreEmbedDataPersonalBest::try_new(pb_idx, &self.origin));

        Ok(ScoreEmbedData {
            score,
            map,
            stars,
            max_combo,
            max_pp,
            replay_score_id,
            miss_analyzer,
            pb_idx,
            global_idx,
            if_fc_pp,
            #[cfg(feature = "twitch")]
            twitch: self.twitch,
        })
    }
}

struct PpAttrs<'m> {
    calc: PpManager<'m>,
    stars: f32,
    max_combo: u32,
    max_pp: f32,
}

impl<'m> PpAttrs<'m> {
    async fn new(
        map: &'m OsuMap,
        mode: GameMode,
        mods: &GameMods,
        grade: Grade,
        lazer: bool,
        pp: Option<f32>,
    ) -> Self {
        let mut calc = Context::pp(map)
            .mode(mode)
            .mods(mods.to_owned())
            .lazer(lazer);

        let mut max_pp = 0.0;
        let mut stars = 0.0;
        let mut max_combo = 0;

        if let Some(attrs) = calc.performance().await {
            max_pp = pp
                .filter(|_| grade.eq_letter(Grade::X) && mode != GameMode::Mania)
                .unwrap_or(attrs.pp() as f32);

            stars = attrs.stars() as f32;
            max_combo = attrs.max_combo();
        }

        Self {
            calc,
            stars,
            max_combo,
            max_pp,
        }
    }
}

#[derive(Copy, Clone)]
pub struct MissAnalyzerCheck {
    guild_id: Option<Id<GuildMarker>>,
}

impl MissAnalyzerCheck {
    pub fn new(guild_id: Option<Id<GuildMarker>>, with_miss_analyzer: bool) -> Self {
        let guild_id = if with_miss_analyzer { guild_id } else { None };

        Self { guild_id }
    }

    pub fn without() -> Self {
        Self { guild_id: None }
    }
}

pub struct ScoreEmbedDataPersonalBest {
    /// Note that `idx` is 0-indexed.
    pub idx: Option<usize>,
    pub formatted: String,
}

pub struct MissAnalyzerData {
    pub score_id: u64,
}

impl ScoreEmbedDataPersonalBest {
    pub fn try_new(pb_idx: PersonalBestIndex, origin: &MessageOrigin) -> Option<Self> {
        let idx = match &pb_idx {
            PersonalBestIndex::FoundScore { idx } | PersonalBestIndex::Presumably { idx } => {
                Some(*idx)
            }
            PersonalBestIndex::FoundBetter { .. }
            | PersonalBestIndex::IfRanked { .. }
            | PersonalBestIndex::NotTop100 => None,
        };

        pb_idx
            .into_embed_description(origin)
            .map(|formatted| Self { idx, formatted })
    }

    /// Note that `idx` should be 0-indexed.
    pub fn from_index(idx: usize) -> Self {
        let origin = MessageOrigin::new(None, Id::new(1));

        let Some(formatted) = PersonalBestIndex::FoundScore { idx }.into_embed_description(&origin)
        else {
            unreachable!()
        };

        Self {
            idx: Some(idx),
            formatted,
        }
    }
}

impl<'q> Searchable<TopCriteria<'q>> for ScoreEmbedDataHalf {
    fn matches(&self, criteria: &FilterCriteria<TopCriteria<'q>>) -> bool {
        let mut matches = true;

        matches &= criteria.combo.contains(self.score.max_combo);
        matches &= criteria.miss.contains(self.score.statistics.miss);
        matches &= criteria.score.contains(self.score.score);
        matches &= criteria.date.contains(self.score.ended_at.date());
        matches &= criteria.stars.contains(self.stars);
        matches &= criteria.pp.contains(self.score.pp);
        matches &= criteria.acc.contains(self.score.accuracy);

        if !criteria.ranked_date.is_empty() {
            let Some(datetime) = self.map.ranked_date() else {
                return false;
            };
            matches &= criteria.ranked_date.contains(datetime.date());
        }

        let attrs = self.map.attributes().mods(self.score.mods.clone()).build();

        matches &= criteria.ar.contains(attrs.ar as f32);
        matches &= criteria.cs.contains(attrs.cs as f32);
        matches &= criteria.hp.contains(attrs.hp as f32);
        matches &= criteria.od.contains(attrs.od as f32);

        let keys = [
            (GameModIntermode::OneKey, 1.0),
            (GameModIntermode::TwoKeys, 2.0),
            (GameModIntermode::ThreeKeys, 3.0),
            (GameModIntermode::FourKeys, 4.0),
            (GameModIntermode::FiveKeys, 5.0),
            (GameModIntermode::SixKeys, 6.0),
            (GameModIntermode::SevenKeys, 7.0),
            (GameModIntermode::EightKeys, 8.0),
            (GameModIntermode::NineKeys, 9.0),
            (GameModIntermode::TenKeys, 10.0),
        ]
        .into_iter()
        .find_map(|(gamemod, keys)| self.score.mods.contains_intermode(gamemod).then_some(keys))
        .unwrap_or(attrs.cs as f32);

        matches &= self.map.mode() != GameMode::Mania || criteria.keys.contains(keys);

        if !matches
            || (criteria.length.is_empty()
                && criteria.bpm.is_empty()
                && criteria.artist.is_empty()
                && criteria.creator.is_empty()
                && criteria.version.is_empty()
                && criteria.title.is_empty()
                && !criteria.has_search_terms())
        {
            return matches;
        }

        let clock_rate = attrs.clock_rate as f32;
        matches &= criteria
            .length
            .contains(self.map.seconds_drain() as f32 / clock_rate);
        matches &= criteria.bpm.contains(self.map.bpm() * clock_rate);

        if !matches
            || (criteria.artist.is_empty()
                && criteria.creator.is_empty()
                && criteria.title.is_empty()
                && criteria.version.is_empty()
                && !criteria.has_search_terms())
        {
            return matches;
        }

        let artist = self.map.artist().cow_to_ascii_lowercase();
        matches &= criteria.artist.matches(&artist);

        let creator = self.map.creator().cow_to_ascii_lowercase();
        matches &= criteria.creator.matches(&creator);

        let version = self.map.version().cow_to_ascii_lowercase();
        matches &= criteria.version.matches(&version);

        let title = self.map.title().cow_to_ascii_lowercase();
        matches &= criteria.title.matches(&title);

        if matches && criteria.has_search_terms() {
            let terms = [artist, creator, version, title];

            matches &= criteria
                .search_terms()
                .all(|term| terms.iter().any(|searchable| searchable.contains(term)))
        }

        matches
    }
}

mod avatar;
mod bws;
mod common;
mod compare;
mod country_snipe_list;
mod country_snipe_stats;
mod leaderboard;
mod map;
mod map_search;
mod match_costs;
mod match_live;
mod medal;
mod medal_stats;
mod medals_missing;
mod most_played;
mod most_played_common;
mod nochoke;
mod osustats_counts;
mod osustats_globals;
mod osustats_list;
mod player_snipe_list;
mod player_snipe_stats;
mod pp_missing;
mod profile;
mod profile_compare;
mod rank;
mod rank_score;
mod ratio;
mod recent;
mod recent_list;
mod simulate;
mod sniped;
mod sniped_difference;
mod top;
mod top_if;
mod top_single;
mod whatif;

pub use avatar::AvatarEmbed;
pub use bws::BWSEmbed;
pub use common::CommonEmbed;
pub use compare::{CompareEmbed, NoScoresEmbed};
pub use country_snipe_list::CountrySnipeListEmbed;
pub use country_snipe_stats::CountrySnipeStatsEmbed;
pub use leaderboard::LeaderboardEmbed;
pub use map::MapEmbed;
pub use map_search::MapSearchEmbed;
pub use match_costs::MatchCostEmbed;
pub use match_live::MatchLiveEmbed;
pub use medal::MedalEmbed;
pub use medal_stats::MedalStatsEmbed;
pub use medals_missing::MedalsMissingEmbed;
pub use most_played::MostPlayedEmbed;
pub use most_played_common::MostPlayedCommonEmbed;
pub use nochoke::NoChokeEmbed;
pub use osustats_counts::OsuStatsCountsEmbed;
pub use osustats_globals::OsuStatsGlobalsEmbed;
pub use osustats_list::OsuStatsListEmbed;
pub use player_snipe_list::PlayerSnipeListEmbed;
pub use player_snipe_stats::PlayerSnipeStatsEmbed;
pub use pp_missing::PPMissingEmbed;
pub use profile::ProfileEmbed;
pub use profile_compare::ProfileCompareEmbed;
pub use rank::RankEmbed;
pub use rank_score::RankRankedScoreEmbed;
pub use ratio::RatioEmbed;
pub use recent::RecentEmbed;
pub use recent_list::RecentListEmbed;
pub use simulate::SimulateEmbed;
pub use sniped::SnipedEmbed;
pub use sniped_difference::SnipedDiffEmbed;
pub use top::TopEmbed;
pub use top_if::TopIfEmbed;
pub use top_single::TopSingleEmbed;
pub use whatif::WhatIfEmbed;

use crate::util::{datetime::sec_to_minsec, numbers::round, BeatmapExt, ScoreExt};

use rosu_v2::prelude::{Beatmap, GameMods};
use std::fmt::Write;

#[inline]
pub fn get_stars(stars: f32) -> String {
    format!("{:.2}★", stars)
}

#[inline]
pub fn get_mods(mods: GameMods) -> String {
    if mods.is_empty() {
        String::new()
    } else {
        format!("+{}", mods)
    }
}

pub fn get_combo(score: impl ScoreExt, map: impl BeatmapExt) -> String {
    let mut combo = String::from("**");
    let _ = write!(combo, "{}x**/", score.max_combo());

    match map.max_combo() {
        Some(amount) => write!(combo, "{}x", amount).unwrap(),
        None => combo.push('-'),
    }

    combo
}

pub fn get_pp(actual: Option<f32>, max: Option<f32>) -> String {
    let mut result = String::with_capacity(17);
    result.push_str("**");

    if let Some(pp) = actual {
        let _ = write!(result, "{:.2}", pp);
    } else {
        result.push('-');
    }

    result.push_str("**/");

    if let Some(max) = max {
        let pp = actual.map(|pp| pp.max(max)).unwrap_or(max);
        let _ = write!(result, "{:.2}", pp);
    } else {
        result.push('-');
    }

    result.push_str("PP");

    result
}

#[inline]
pub fn get_keys(mods: GameMods, map: &Beatmap) -> String {
    if let Some(key_mod) = mods.has_key_mod() {
        format!("[{}]", key_mod)
    } else {
        format!("[{}K]", map.cs as u32)
    }
}

#[inline]
pub fn get_map_info(map: &Beatmap) -> String {
    format!(
        "Length: `{}` (`{}`) BPM: `{}` Objects: `{}`\n\
        CS: `{}` AR: `{}` OD: `{}` HP: `{}` Stars: `{}`",
        sec_to_minsec(map.seconds_total),
        sec_to_minsec(map.seconds_drain),
        round(map.bpm),
        map.count_objects(),
        round(map.cs),
        round(map.ar),
        round(map.od),
        round(map.hp),
        round(map.stars)
    )
}

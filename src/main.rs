use core::task;
use std::{collections::HashMap, sync::Arc};

use chrono::{Datelike, FixedOffset, NaiveDate, TimeZone, Utc};
use poise::serenity_prelude::{self as serenity, ChannelId, GuildId};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

static FILE_LOCK: Mutex<()> = Mutex::const_new(());
static FILE_PATH: &str = "birthdays.json";
static LIFE_EXPECTANCY: i32 = 83;
static CHECK_TIME: u64 = 60 * 60; // 1 hour

struct Data {} // User data, which is stored and accessible in all command invocations
type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

#[derive(Debug, Serialize, Deserialize, Default)]
struct BirthdayList {
    entries: Vec<BirthdayEntry>,
    server_channels: HashMap<GuildId, ChannelId>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BirthdayEntry {
    user_id: serenity::UserId,
    guild_id: GuildId,
    name: String,
    date: NaiveDate,
    last_announcement: Option<NaiveDate>,
    utc_offset: i32,
}

async fn read_from_file() -> Result<BirthdayList, Error> {
    let _lock = FILE_LOCK.lock().await;
    let data = std::fs::read_to_string(FILE_PATH);
    // Make a backup of the file if it's corrupted and return an empty list
    let data = match data {
        Ok(data) => data,
        Err(_) => {
            let backup_path = format!("{}.bak", FILE_PATH);
            let _ = std::fs::copy(FILE_PATH, &backup_path);
            panic!("Corrupted file, backed up to {}", backup_path);
        }
    };
    Ok(serde_json::from_str(&data).unwrap_or_default())
}

async fn write_to_file(birthdays: &BirthdayList) -> Result<(), Error> {
    let _lock = FILE_LOCK.lock().await;
    let data = serde_json::to_string_pretty(birthdays)?;
    std::fs::write(FILE_PATH, data)?;
    Ok(())
}

fn args_to_date(day: usize, month: usize, year: Option<usize>) -> Result<NaiveDate, Error> {
    match NaiveDate::from_ymd_opt(year.unwrap_or(2024) as i32, month as u32, day as u32) {
        Some(date) => Ok(date),
        None => Err("Invalid date!".into()),
    }
}

async fn append_birthday(
    user_id: serenity::UserId,
    guild_id: GuildId,
    name: String,
    day: usize,
    month: usize,
    year: Option<usize>,
    utc_offset: i32,
) -> Result<(), Error> {
    let mut birthdays = read_from_file().await?;
    // Remove any existing entry for this user and this specific guild
    birthdays
        .entries
        .retain(|entry| entry.user_id != user_id || entry.guild_id != guild_id);

    // Add the new entry
    birthdays.entries.push(BirthdayEntry {
        user_id,
        guild_id,
        name,
        date: args_to_date(day, month, year)?,
        last_announcement: None,
        utc_offset,
    });
    write_to_file(&birthdays).await?;
    Ok(())
}

fn date_to_discord_timestamp(date: NaiveDate, offset: i32, relative: bool) -> String {
    let flag = if relative { "R" } else { "f" };

    // Calculate time with offset
    let offset = if offset == 0 { 0 } else { offset - 1 };
    let date = date.and_hms_opt(0, 0, 0).unwrap() - chrono::Duration::hours(offset as i64);
    let timestamp = date.and_utc().timestamp();

    format!("<t:{}:{}>", timestamp, flag)
}

async fn get_birthday_from_file(
    user_id: serenity::UserId,
    guild_id: GuildId,
) -> Result<Option<BirthdayEntry>, Error> {
    let birthdays = read_from_file().await?;
    Ok(birthdays
        .entries
        .into_iter()
        .find(|entry| entry.user_id == user_id && entry.guild_id == guild_id))
}

fn offset_to_string(offset: i32) -> String {
    if offset >= 0 {
        format!("+{}", offset)
    } else {
        format!("{}", offset)
    }
}

/// Sets your or another user's birthday
#[poise::command(slash_command, prefix_command)]
async fn set_birthday(
    ctx: Context<'_>,
    #[description = "Day"] day: usize,
    #[description = "Month"] month: usize,
    #[description = "Year"] year: Option<usize>,
    #[description = "UTC offset from UTC+00"] utc_offset: i32,
    #[description = "User to set the birthday for (defaults to yourself)"] user: Option<
        serenity::User,
    >,
) -> Result<(), Error> {
    // Turn the month and day into a date and check if it's valid
    if chrono::NaiveDate::from_ymd_opt(year.unwrap_or(2024) as i32, month as u32, day as u32)
        .is_none()
    {
        ctx.say("🐺🎩❌ Invalid date!").await?;
        return Ok(());
    }

    let user = user.unwrap_or_else(|| ctx.author().clone());
    append_birthday(
        user.id,
        ctx.guild_id().unwrap(),
        user.name.clone(),
        day,
        month,
        year,
        utc_offset,
    )
    .await?;

    ctx.say(format!(
        "✍️📅🎈 Added birthday for {} on {}.{} (UTC{}) which is {} for you!",
        user.name,
        day,
        month,
        offset_to_string(utc_offset),
        date_to_discord_timestamp(args_to_date(day, month, year)?, utc_offset, false)
    ))
    .await?;
    Ok(())
}

/// Gets your or another user's birthday
#[poise::command(slash_command, prefix_command)]
async fn get_birthday(
    ctx: Context<'_>,
    #[description = "User to get the birthday for (defaults to yourself)"] user: Option<
        serenity::User,
    >,
) -> Result<(), Error> {
    let user = user.unwrap_or_else(|| ctx.author().clone());
    let entry = get_birthday_from_file(user.id, ctx.guild_id().unwrap()).await?;
    let entry = match entry {
        Some(entry) => entry,
        None => {
            ctx.say("☹️🎈 No birthday set for this user for this guild!")
                .await?;
            return Ok(());
        }
    };

    // Check whether the birthday already happened this year
    let today = Utc::now().naive_utc().date();

    // Set entry year to this year
    let entry = BirthdayEntry {
        date: NaiveDate::from_ymd_opt(today.year(), entry.date.month(), entry.date.day()).unwrap(),
        ..entry
    };

    // If the birthday already happened this year, set the year to next year
    let year = if today > entry.date {
        today.year() + 1
    } else {
        today.year()
    };

    // Get next birthday
    let next_birthday =
        NaiveDate::from_ymd_opt(year, entry.date.month(), entry.date.day()).unwrap();

    ctx.say(format!(
        "📅🎈 {}'s birthday is on {}.{} (UTC{}) so {} which is {} for you!",
        entry.name,
        entry.date.day(),
        entry.date.month(),
        offset_to_string(entry.utc_offset),
        date_to_discord_timestamp(next_birthday, entry.utc_offset, true),
        date_to_discord_timestamp(next_birthday, entry.utc_offset, false),
    ))
    .await?;
    Ok(())
}

#[poise::command(slash_command, prefix_command, required_permissions = "MANAGE_GUILD")]
async fn set_announcement_channel(
    ctx: Context<'_>,
    #[description = "Channel to set as the birthday announcement channel"] channel: ChannelId,
) -> Result<(), Error> {
    let mut birthdays = read_from_file().await?;
    birthdays
        .server_channels
        .insert(ctx.guild_id().unwrap(), channel);
    write_to_file(&birthdays).await?;
    ctx.say(format!("📢🎈 Birthday channel set to <#{}>!", channel))
        .await?;
    Ok(())
}

/// Gets your or another user's birthday
#[poise::command(slash_command, prefix_command)]
async fn time_left(
    ctx: Context<'_>,
    #[description = "User to get the skibidi for (defaults to yourself)"] user: Option<
        serenity::User,
    >,
) -> Result<(), Error> {
    let user = user.unwrap_or_else(|| ctx.author().clone());
    let entry = get_birthday_from_file(user.id, ctx.guild_id().unwrap()).await?;
    let entry = match entry {
        Some(entry) => entry,
        None => {
            ctx.say("☹️🎈 No birthday set for this user for this guild!")
                .await?;
            return Ok(());
        }
    };

    if entry.date.year() == 2024 {
        ctx.say("🐺🎩❌ Can't calculate skibidi (User has not set year)!")
            .await?;
        return Ok(());
    }

    // Check whether the birthday already happened this year
    let entry = BirthdayEntry {
        date: NaiveDate::from_ymd_opt(
            entry.date.year() + LIFE_EXPECTANCY,
            entry.date.month(),
            entry.date.day(),
        )
        .unwrap(),
        ..entry
    };

    ctx.say(format!(
        "💀 {} is expected to skibidi out of this world {} (🇩🇪 avg)",
        entry.name,
        date_to_discord_timestamp(entry.date, entry.utc_offset, true)
    ))
    .await?;
    Ok(())
}

async fn check_for_announcements(context: Arc<serenity::Http>) {
    println!("Checking for birthdays...");

    loop {
        let mut birthdays = read_from_file().await.unwrap();

        // Lock the file to prevent overwrites while we're checking
        {
            let _ = FILE_LOCK.lock().await;

            let today = Utc::now().naive_utc().date();
            for entry in birthdays.entries.iter_mut() {
                let offset_entry = entry.date - chrono::Duration::hours(entry.utc_offset as i64);
                if offset_entry.month() == today.month()
                    && offset_entry.day() == today.day()
                    && (entry.last_announcement.is_none()
                        || entry.last_announcement.unwrap().year() != today.year())
                {
                    let channel = birthdays.server_channels.get(&entry.guild_id);
                    if let Some(channel) = channel {
                        let channel = channel.clone();
                        channel
                            .say(
                                &context,
                                format!("🎉🎈 Happy Birthday {}! 🎈🎉", entry.name),
                            )
                            .await
                            .unwrap();
                    }

                    entry.last_announcement = Some(today);
                }
            }

            let data = serde_json::to_string_pretty(&birthdays).unwrap();
            std::fs::write(FILE_PATH, data).unwrap();
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(CHECK_TIME)).await;
    }
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().unwrap();
    let token = std::env::var("DISCORD_TOKEN").expect("missing DISCORD_TOKEN");
    let intents = serenity::GatewayIntents::non_privileged();

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                set_birthday(),
                get_birthday(),
                time_left(),
                set_announcement_channel(),
            ],
            ..Default::default()
        })
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                tokio::spawn(check_for_announcements(ctx.http.clone()));
                poise::builtins::register_globally(
                    ctx.clone(),
                    &framework.options().commands
                )
                .await?;
                Ok(Data {})
            })
        })
        .build();

    let client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await;
    client.unwrap().start().await.unwrap();
}

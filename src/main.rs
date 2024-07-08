use std::collections::HashMap;

use chrono::{Datelike, NaiveDate, Utc};
use poise::serenity_prelude::{self as serenity, ChannelId, GuildId};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

static FILE_LOCK: Mutex<()> = Mutex::const_new(());
static FILE_PATH: &str = "birthdays.json";
static LIFE_EXPECTANCY: i32 = 83;

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
            return Ok(BirthdayList::default());
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
    if day < 1 || day > 31 || month < 1 || month > 12 {
        return Err("Invalid date!".into());
    }
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
    });
    write_to_file(&birthdays).await?;
    Ok(())
}

fn date_to_discord_timestamp(date: NaiveDate, relative: bool) -> String {
    let flag = if relative { "R" } else { "D" };
    format!(
        "<t:{}:{}>",
        date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp(),
        flag
    )
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

/// Sets your or another user's birthday
#[poise::command(slash_command, prefix_command)]
async fn set_birthday(
    ctx: Context<'_>,
    #[description = "Day"] day: usize,
    #[description = "Month"] month: usize,
    #[description = "Year"] year: Option<usize>,
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
    )
    .await?;
    ctx.say(format!(
        "✍️📅🎈 Added birthday for {} on {}.{} (UTC {})!",
        user.name, day, month, date_to_discord_timestamp(args_to_date(day, month, year)?, false)
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
        "📅🎈 {}'s birthday is on {}.{} ({} UTC) so {}!",
        entry.name,
        entry.date.day(),
        entry.date.month(),
        date_to_discord_timestamp(next_birthday, false),
        date_to_discord_timestamp(next_birthday, true)
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
        ctx.say("🐺🎩❌ Can't calculate skibidi (User has not set year)!").await?;
        return Ok(());
    }

    // Check whether the birthday already happened this year
    let entry = BirthdayEntry {
        date: NaiveDate::from_ymd_opt(entry.date.year() + LIFE_EXPECTANCY, entry.date.month(), entry.date.day()).unwrap(),
        ..entry
    };

    ctx.say(format!(
        "💀 {} is expected to skibidi out of this world {} (🇩🇪 avg)",
        entry.name,
        date_to_discord_timestamp(entry.date, true)
    ))
    .await?;
    Ok(())
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().unwrap();
    let token = std::env::var("DISCORD_TOKEN").expect("missing DISCORD_TOKEN");
    let intents = serenity::GatewayIntents::non_privileged();

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![set_birthday(), get_birthday(), time_left(), set_announcement_channel()],
            ..Default::default()
        })
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_in_guild(ctx, &framework.options().commands, GuildId::from(477891535174631424)).await?;
                Ok(Data {})
            })
        })
        .build();

    let client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await;
    client.unwrap().start().await.unwrap();
}

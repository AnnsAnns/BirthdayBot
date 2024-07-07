use chrono::Datelike;
use poise::serenity_prelude::{self as serenity, GuildId};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

static FILE_LOCK: Mutex<()> = Mutex::const_new(());
static FILE_PATH: &str = "birthdays.json";

struct Data {} // User data, which is stored and accessible in all command invocations
type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

#[derive(Debug, Serialize, Deserialize, Default)]
struct BirthdayList {
    entries: Vec<BirthdayEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BirthdayEntry {
    user_id: serenity::UserId,
    guild_id: GuildId,
    name: String,
    day: usize,
    month: usize,
    year: Option<usize>,
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

async fn append_birthday(
    user_id: serenity::UserId,
    guild_id: GuildId,
    name: String,
    day: usize,
    month: usize,
    year: Option<usize>,
) -> Result<(), Error> {
    let mut birthdays = read_from_file().await?;
    // Remove any existing entry for this user
    birthdays.entries.retain(|entry| entry.user_id != user_id && entry.guild_id != guild_id);

    // Add the new entry
    birthdays.entries.push(BirthdayEntry {
        user_id,
        guild_id,
        name,
        day,
        month,
        year,
    });
    write_to_file(&birthdays).await?;
    Ok(())
}

async fn get_birthday_from_file(user_id: serenity::UserId, guild_id: GuildId) -> Result<Option<BirthdayEntry>, Error> {
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
    if day < 1 || day > 31 || month < 1 || month > 12 {
        ctx.say("🐺🎩❌ Invalid date!").await?;
        return Ok(());
    }

    let user = user.unwrap_or_else(|| ctx.author().clone());
    append_birthday(user.id, ctx.guild_id().unwrap(), user.name.clone(), day, month, year).await?;
    let year = match year {
        Some(year) => year.to_string(),
        None => "???".to_string(),
    };
    ctx.say(format!(
        "✍️📅🎈 Added birthday for {} on {}.{}.{}!",
        user.name, day, month, year
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
    let entry = get_birthday_from_file(user.id, ctx.guild_id().unwrap_or(GuildId::from(0))).await?;
    let entry = match entry {
        Some(entry) => entry,
        None => {
            ctx.say("☹️🎈 No birthday set for this user for this guild!").await?;
            return Ok(());
        }
    };
    let year = match entry.year {
        Some(year) => {
            let age = chrono::Local::now().year() - year as i32;
            format!(".{} - They are {} years old", year, age)
        }
        None => "".to_string(),
    };
    ctx.say(format!(
        "📅🎈 {}'s birthday is on {}.{}{}!",
        entry.name, entry.day, entry.month, year
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
            commands: vec![set_birthday(), get_birthday()],
            ..Default::default()
        })
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(
                    ctx,
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

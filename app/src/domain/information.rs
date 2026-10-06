use crate::domain::call::GetInformationSupported;
use crate::vocabulary::{fill_in, vocabulary};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const BERLIN_WEATHER_URL: &str =
    "https://api.open-meteo.com/v1/forecast?latitude=52.52&longitude=13.41&current_weather=true";
const EUR_EXCHANGE_RATE_URL: &str =
    "https://bank.gov.ua/NBUStatService/v1/statdirectory/exchange?valcode=EUR&json";
const TIMEOUT: Duration = Duration::from_secs(5);

impl GetInformationSupported {
    /// Fetches the facts the response formulator answers from, worded as the vocabulary has them.
    pub async fn fetch(self, http: &Client) -> anyhow::Result<String> {
        let (facts, instructions) = (&vocabulary().facts, &vocabulary().instructions);

        match self {
            // In words: from a weather code and a number of degrees the formulator made a sky and a wind up.
            GetInformationSupported::GetCurrentWeatherInBerlin => {
                let forecast = get_json(http, BERLIN_WEATHER_URL).await?;
                let now = &forecast["current_weather"];
                let (Some(code), Some(temperature), Some(windspeed), Some(degrees)) = (
                    now["weathercode"].as_u64(),
                    now["temperature"].as_f64(),
                    now["windspeed"].as_f64(),
                    now["winddirection"].as_f64(),
                ) else {
                    anyhow::bail!("No current weather in {forecast}");
                };

                Ok(fill_in(
                    &facts.weather,
                    &[
                        ("sky", facts.sky(code)),
                        ("temperature", temperature.to_string().as_str()),
                        ("windspeed", windspeed.to_string().as_str()),
                        ("direction", facts.direction(degrees)),
                    ],
                ))
            }
            GetInformationSupported::GetCurrentUAHPerEUR => {
                let rates = get_json(http, EUR_EXCHANGE_RATE_URL).await?;
                let rate = &rates[0];
                anyhow::ensure!(rate["rate"].is_number(), "No EUR rate in {rates}");

                Ok(fill_in(
                    &facts.exchange_rate,
                    &[
                        ("rate", rate["rate"].to_string().as_str()),
                        ("date", rate["exchangedate"].as_str().unwrap_or(&facts.today)),
                    ],
                ))
            }
            // Nothing to fetch for these two: the first is turned down, the second is answered from the form state.
            GetInformationSupported::CalendarHelp => Ok(instructions.calendar_help.clone()),
            GetInformationSupported::FormInformation => Ok(instructions.form_question.clone()),
            // Not fetched either: the main menu answers it from the call's hints, see `HintMap::last_filled_out_form_information`.
            GetInformationSupported::LastFilledOutFormInformation => anyhow::bail!("{self} is answered from the hints of the call"),
        }
    }
}

async fn get_json(http: &Client, url: &str) -> anyhow::Result<Value> {
    Ok(http
        .get(url)
        .timeout(TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

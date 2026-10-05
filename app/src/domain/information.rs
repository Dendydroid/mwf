use crate::domain::call::GetInformationSupported;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const BERLIN_WEATHER_URL: &str =
    "https://api.open-meteo.com/v1/forecast?latitude=52.52&longitude=13.41&current_weather=true";
const EUR_EXCHANGE_RATE_URL: &str =
    "https://bank.gov.ua/NBUStatService/v1/statdirectory/exchange?valcode=EUR&json";
const TIMEOUT: Duration = Duration::from_secs(5);

impl GetInformationSupported {
    /// Fetches the facts the response formulator answers from.
    pub async fn fetch(self, http: &Client) -> anyhow::Result<String> {
        match self {
            GetInformationSupported::GetCurrentWeatherInBerlin => {
                let forecast = get_json(http, BERLIN_WEATHER_URL).await?;

                Ok(format!(
                    "Current weather in Berlin: {} (units: {}, weathercode is a WMO weather code)",
                    forecast["current_weather"], forecast["current_weather_units"]
                ))
            }
            GetInformationSupported::GetCurrentUAHPerEUR => {
                let rates = get_json(http, EUR_EXCHANGE_RATE_URL).await?;
                let rate = &rates[0];
                anyhow::ensure!(rate["rate"].is_number(), "No EUR rate in {rates}");

                Ok(format!(
                    "1 EUR = {} UAH, the official National Bank of Ukraine rate for {}",
                    rate["rate"],
                    rate["exchangedate"].as_str().unwrap_or("today")
                ))
            }
            // Nothing to fetch for these two: the first is turned down, the second is answered from the form state.
            GetInformationSupported::CalendarHelp => Ok("The caller asked about dates or the calendar, which you cannot help with. Say you are sorry that you cannot help with dates".into()),
            GetInformationSupported::FormInformation => Ok("The caller asked about the form. Answer it in one sentence, then say the next step in a sentence of its own.".to_string()),
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

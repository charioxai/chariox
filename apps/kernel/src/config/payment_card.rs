//! MP-08 / MP-09 / MP-10 / MP-11: a payment card is one structured Vault value.
//! The kernel is the only reader and writer of its format. Errors are fixed
//! sentences, so nothing that was typed reaches a message, log or result.
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::error::DaemonError;

/// The secret card, as stored in the Vault. Never serialized outside the Vault value.
#[derive(Deserialize, Serialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub(crate) struct PaymentCard {
    number: String,
    expiry_month: u8,
    expiry_year: u16,
    security_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

impl std::fmt::Debug for PaymentCard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PaymentCard([REDACTED])")
    }
}

/// What the owner's list shows about a card; nothing here completes a payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentCardSummary {
    pub last4: String,
    pub expiry_month: u8,
    pub expiry_year: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
}

/// One page field of a card checkout, filled by its own browser insertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentCardField {
    Number,
    /// `MM/YY`, the common single expiry box.
    Expiry,
    ExpiryMonth,
    ExpiryYear,
    SecurityCode,
    Name,
}

fn invalid(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "vault_payment_card",
        message: message.into(),
    }
}

fn luhn(number: &str) -> bool {
    let sum: u32 = number
        .bytes()
        .rev()
        .enumerate()
        .map(|(index, byte)| {
            let digit = u32::from(byte - b'0');
            match (index % 2, digit * 2) {
                (0, _) => digit,
                (_, doubled) if doubled > 9 => doubled - 9,
                (_, doubled) => doubled,
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn digits(value: &str, lengths: std::ops::RangeInclusive<usize>) -> bool {
    lengths.contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

impl PaymentCard {
    /// Parses the owner's card. `now_ms` decides whether it has expired.
    pub(crate) fn parse(value: &str, now_ms: u64) -> Result<Self, DaemonError> {
        let card: Self = serde_json::from_str(value)
            .map_err(|_| invalid("Enter the card number, expiry and security code."))?;
        if !digits(&card.number, 12..=19) || !luhn(&card.number) {
            return Err(invalid("Enter a valid card number."));
        }
        let now = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
            i64::try_from(now_ms).unwrap_or(i64::MAX),
        )
        .unwrap_or_default();
        let current = (
            u16::try_from(chrono::Datelike::year(&now)).unwrap_or_default(),
            u8::try_from(chrono::Datelike::month(&now)).unwrap_or_default(),
        );
        if !(1..=12).contains(&card.expiry_month)
            || !(2000..=2099).contains(&card.expiry_year)
            || (card.expiry_year, card.expiry_month) < current
        {
            return Err(invalid("Enter an expiry date that has not passed."));
        }
        if !digits(&card.security_code, 3..=4) {
            return Err(invalid("Enter the 3 or 4 digit security code."));
        }
        if card.name.as_deref().is_some_and(|name| {
            name.trim().is_empty()
                || name.chars().count() > 100
                || name.chars().any(char::is_control)
        }) {
            return Err(invalid("Enter the name on the card."));
        }
        Ok(card)
    }

    /// The canonical stored form, so every reader sees the same fields.
    pub(crate) fn canonical(&self) -> Zeroizing<String> {
        Zeroizing::new(serde_json::to_string(self).expect("a card serializes"))
    }

    pub(crate) fn summary(&self) -> PaymentCardSummary {
        let brand = match self.number.as_bytes() {
            [b'4', ..] => Some("Visa"),
            [b'3', b'4' | b'7', ..] => Some("American Express"),
            [b'5', b'1'..=b'5', ..] => Some("Mastercard"),
            _ => self
                .number
                .get(..4)
                .and_then(|prefix| prefix.parse::<u16>().ok())
                .filter(|prefix| (2221..=2720).contains(prefix))
                .map(|_| "Mastercard"),
        };
        PaymentCardSummary {
            last4: self.number[self.number.len() - 4..].into(),
            expiry_month: self.expiry_month,
            expiry_year: self.expiry_year,
            name: self.name.clone(),
            brand: brand.map(str::to_owned),
        }
    }

    /// The text one checkout field receives; none for a name the card was saved without.
    pub(crate) fn field(&self, field: PaymentCardField) -> Option<Zeroizing<String>> {
        Some(Zeroizing::new(match field {
            PaymentCardField::Number => self.number.clone(),
            PaymentCardField::Expiry => {
                format!("{:02}/{:02}", self.expiry_month, self.expiry_year % 100)
            }
            PaymentCardField::ExpiryMonth => format!("{:02}", self.expiry_month),
            PaymentCardField::ExpiryYear => self.expiry_year.to_string(),
            PaymentCardField::SecurityCode => self.security_code.clone(),
            PaymentCardField::Name => self.name.clone()?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Published payment-provider test cards, never real ones.
    const VISA: &str = "4242424242424242";
    const AMEX: &str = "378282246310005";
    const MASTERCARD_2: &str = "2223003122003222";
    // 2030-06-15T00:00:00Z
    const NOW_MS: u64 = 1_907_712_000_000;

    fn card(number: &str, month: u8, year: u16, code: &str, name: &str) -> String {
        serde_json::json!({"number": number, "expiry_month": month, "expiry_year": year,
            "security_code": code, "name": name})
        .to_string()
    }

    #[test]
    fn parse_accepts_valid_cards_and_refuses_everything_else_without_echo() {
        for valid in [
            card(VISA, 6, 2030, "123", "Ada Lovelace"),
            card(AMEX, 1, 2031, "1234", "Ada Lovelace"),
        ] {
            assert!(PaymentCard::parse(&valid, NOW_MS).is_ok());
        }
        for invalid in [
            card("4242424242424241", 6, 2030, "123", "Ada"),
            card("4242-4242-4242-4242", 6, 2030, "123", "Ada"),
            card("42424242424", 6, 2030, "123", "Ada"),
            card(VISA, 5, 2030, "123", "Ada"),
            card(VISA, 13, 2030, "123", "Ada"),
            card(VISA, 6, 2100, "123", "Ada"),
            card(VISA, 6, 2030, "12", "Ada"),
            card(VISA, 6, 2030, "12a", "Ada"),
            card(VISA, 6, 2030, "123", " "),
            card(VISA, 6, 2030, "123", "Ada\nLovelace"),
            card(VISA, 6, 2030, "123", "Ada").replace("}", ",\"pin\":\"9\"}"),
            "4242424242424242".into(),
        ] {
            let error = PaymentCard::parse(&invalid, NOW_MS).unwrap_err();
            let text = format!("{error} {error:?}");
            assert!(!text.contains("4242") && !text.contains("123"), "{text}");
        }
    }

    #[test]
    fn canonical_summary_and_fields_come_from_one_parsed_card() {
        let parsed = PaymentCard::parse(&card(VISA, 6, 2030, "123", "Ada"), NOW_MS).unwrap();
        assert!(!format!("{parsed:?}").contains(VISA));
        let again = PaymentCard::parse(&parsed.canonical(), NOW_MS).unwrap();
        assert_eq!(again.summary(), parsed.summary());
        assert_eq!(
            serde_json::to_value(parsed.summary()).unwrap(),
            serde_json::json!({"last4": "4242", "expiry_month": 6, "expiry_year": 2030,
                "name": "Ada", "brand": "Visa"})
        );
        let field = |field| parsed.field(field).unwrap().to_string();
        assert_eq!(field(PaymentCardField::Number), VISA);
        assert_eq!(field(PaymentCardField::Expiry), "06/30");
        assert_eq!(field(PaymentCardField::ExpiryMonth), "06");
        assert_eq!(field(PaymentCardField::ExpiryYear), "2030");
        assert_eq!(field(PaymentCardField::SecurityCode), "123");
        assert_eq!(field(PaymentCardField::Name), "Ada");
        let brand = |number: &str| {
            PaymentCard::parse(&card(number, 6, 2030, "1234", "Ada"), NOW_MS)
                .unwrap()
                .summary()
                .brand
        };
        assert_eq!(brand(AMEX).as_deref(), Some("American Express"));
        assert_eq!(brand(MASTERCARD_2).as_deref(), Some("Mastercard"));
        assert_eq!(brand("6011111111111117"), None);
    }

    #[test]
    fn the_name_on_the_card_is_optional() {
        let unnamed = serde_json::json!({"number": VISA, "expiry_month": 6,
            "expiry_year": 2030, "security_code": "123"})
        .to_string();
        let parsed = PaymentCard::parse(&unnamed, NOW_MS).unwrap();
        assert_eq!(
            serde_json::to_value(parsed.summary()).unwrap(),
            serde_json::json!({"last4": "4242", "expiry_month": 6, "expiry_year": 2030,
                "brand": "Visa"})
        );
        let again = PaymentCard::parse(&parsed.canonical(), NOW_MS).unwrap();
        assert_eq!(again.summary(), parsed.summary());
        assert!(parsed.field(PaymentCardField::Name).is_none());
    }
}

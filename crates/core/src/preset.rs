use std::str::FromStr;

use crate::error::SettingsError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub long_edge: u32,
    pub quality: u8,
}

impl Settings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.long_edge == 0 {
            return Err(SettingsError::LongEdge(self.long_edge));
        }
        if !(1..=100).contains(&self.quality) {
            return Err(SettingsError::Quality(self.quality));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Low,
    Default,
    High,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Low, Preset::Default, Preset::High];

    pub fn settings(self) -> Settings {
        match self {
            Preset::Low => Settings {
                long_edge: 1600,
                quality: 72,
            },
            Preset::Default => Settings {
                long_edge: 2400,
                quality: 82,
            },
            Preset::High => Settings {
                long_edge: 3200,
                quality: 90,
            },
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Preset::Low => "low",
            Preset::Default => "default",
            Preset::High => "high",
        }
    }
}

impl FromStr for Preset {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Preset::ALL.into_iter().find(|p| p.name() == s).ok_or(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_match_requirements() {
        let values: Vec<(u32, u8)> = Preset::ALL
            .iter()
            .map(|p| (p.settings().long_edge, p.settings().quality))
            .collect();
        assert_eq!(values, [(1600, 72), (2400, 82), (3200, 90)]);
    }

    #[test]
    fn preset_names_round_trip() {
        for preset in Preset::ALL {
            assert_eq!(preset.name().parse(), Ok(preset));
        }
        assert_eq!("medium".parse::<Preset>(), Err(()));
    }

    #[test]
    fn rejects_out_of_range_settings() {
        let ok = Settings {
            long_edge: 1,
            quality: 1,
        };
        assert_eq!(ok.validate(), Ok(()));
        assert_eq!(
            Settings {
                long_edge: 0,
                quality: 80
            }
            .validate(),
            Err(SettingsError::LongEdge(0))
        );
        for quality in [0, 101] {
            assert_eq!(
                Settings {
                    long_edge: 100,
                    quality
                }
                .validate(),
                Err(SettingsError::Quality(quality))
            );
        }
    }
}

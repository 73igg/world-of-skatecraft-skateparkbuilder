//! Private, server-owned Skate return flags. Each message belongs to the authenticated character.
pub const PREFIX: &str = "SKATEFLAG\t1\t";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flag {
    pub revision: u64,
    pub map: u32,
    pub instance: u32,
    pub zone: u32,
    pub position: [f32; 3],
    pub facing: f32,
}

impl Flag {
    pub fn valid(self) -> bool {
        self.revision > 0
            && self.position.iter().all(|x| x.is_finite())
            && self.facing.is_finite()
            && (0.0..std::f32::consts::TAU).contains(&self.facing)
    }
    fn fields(self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.revision,
            self.map,
            self.instance,
            self.zone,
            self.position[0],
            self.position[1],
            self.position[2],
            self.facing
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Message {
    Hello,
    Place {
        request: u32,
        position: [f32; 3],
        facing: f32,
    },
    Return {
        request: u32,
        revision: u64,
    },
    Snapshot {
        request: u32,
        flag: Option<Flag>,
    },
    Returning {
        request: u32,
        flag: Flag,
        seamless: bool,
    },
    Rejected {
        request: u32,
        reason: u8,
    },
}

impl Message {
    pub fn encode(self) -> Result<String, &'static str> {
        let body = match self {
            Self::Hello => "H".into(),
            Self::Place {
                request,
                position,
                facing,
            } => {
                if request == 0
                    || !position.iter().all(|x| x.is_finite())
                    || !facing.is_finite()
                    || !(0.0..std::f32::consts::TAU).contains(&facing)
                {
                    return Err("invalid placement");
                }
                format!(
                    "S\t{request}\t{}\t{}\t{}\t{facing}",
                    position[0], position[1], position[2]
                )
            }
            Self::Return { request, revision } => {
                if request == 0 {
                    return Err("invalid return");
                }
                format!("R\t{request}\t{revision}")
            }
            Self::Snapshot {
                request,
                flag: None,
            } => format!("F\t{request}\t0"),
            Self::Snapshot {
                request,
                flag: Some(f),
            } => {
                if !f.valid() {
                    return Err("invalid saved flag");
                }
                format!("F\t{request}\t{}", f.fields())
            }
            Self::Returning {
                request,
                flag,
                seamless,
            } => {
                if request == 0 || !flag.valid() {
                    return Err("invalid return target");
                }
                format!("T\t{request}\t{}\t{}", u8::from(seamless), flag.fields())
            }
            Self::Rejected { request, reason } => format!("E\t{request}\t{reason}"),
        };
        let text = format!("{PREFIX}{body}");
        if text.len() > 255 {
            return Err("message too long");
        }
        Ok(text)
    }

    pub fn decode(text: &str) -> Result<Self, &'static str> {
        if text.len() > 255 {
            return Err("message too long");
        }
        let p: Vec<_> = text
            .strip_prefix(PREFIX)
            .ok_or("wrong protocol")?
            .split('\t')
            .collect();
        let u = |i: usize| -> Result<u32, &'static str> {
            p.get(i)
                .ok_or("missing field")?
                .parse()
                .map_err(|_| "invalid integer")
        };
        let big = |i: usize| -> Result<u64, &'static str> {
            p.get(i)
                .ok_or("missing field")?
                .parse()
                .map_err(|_| "invalid integer")
        };
        let number = |i: usize| -> Result<f32, &'static str> {
            let x: f32 = p
                .get(i)
                .ok_or("missing field")?
                .parse()
                .map_err(|_| "invalid number")?;
            if x.is_finite() {
                Ok(x)
            } else {
                Err("non-finite number")
            }
        };
        let flag = |i: usize| -> Result<Flag, &'static str> {
            let f = Flag {
                revision: big(i)?,
                map: u(i + 1)?,
                instance: u(i + 2)?,
                zone: u(i + 3)?,
                position: [number(i + 4)?, number(i + 5)?, number(i + 6)?],
                facing: number(i + 7)?,
            };
            if f.valid() {
                Ok(f)
            } else {
                Err("invalid flag")
            }
        };
        let message = match (p[0], p.len()) {
            ("H", 1) => Self::Hello,
            ("S", 6) => Self::Place {
                request: u(1)?,
                position: [number(2)?, number(3)?, number(4)?],
                facing: number(5)?,
            },
            ("R", 3) => Self::Return {
                request: u(1)?,
                revision: big(2)?,
            },
            ("F", 3) if p[2] == "0" => Self::Snapshot {
                request: u(1)?,
                flag: None,
            },
            ("F", 10) => Self::Snapshot {
                request: u(1)?,
                flag: Some(flag(2)?),
            },
            ("T", 11) if matches!(p[2], "0" | "1") => Self::Returning {
                request: u(1)?,
                seamless: p[2] == "1",
                flag: flag(3)?,
            },
            ("E", 3) => Self::Rejected {
                request: u(1)?,
                reason: u(2)?.try_into().map_err(|_| "invalid reason")?,
            },
            _ => return Err("invalid message shape"),
        };
        message.encode()?;
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_flag_requests_and_exact_saved_coordinates_round_trip() {
        let flag = Flag {
            revision: 7,
            map: 0,
            instance: 0,
            zone: 12,
            position: [-8949.173, -132.4567, 83.91234],
            facing: 5.719,
        };
        for m in [
            Message::Hello,
            Message::Place {
                request: 1,
                position: flag.position,
                facing: flag.facing,
            },
            Message::Return {
                request: 2,
                revision: 7,
            },
            Message::Snapshot {
                request: 0,
                flag: None,
            },
            Message::Snapshot {
                request: 1,
                flag: Some(flag),
            },
            Message::Returning {
                request: 2,
                flag,
                seamless: true,
            },
            Message::Returning {
                request: 2,
                flag,
                seamless: false,
            },
            Message::Rejected {
                request: 1,
                reason: 3,
            },
        ] {
            assert_eq!(Message::decode(&m.encode().unwrap()), Ok(m));
        }
    }
    #[test]
    fn malformed_unsupported_and_nonfinite_requests_are_rejected() {
        for text in [
            "",
            "SKATEFLAG\t2\tH",
            "SKATEFLAG\t1\tH\tx",
            "SKATEFLAG\t1\tS\t1\tNaN\t0\t0\t0",
            "SKATEFLAG\t1\tS\t0\t0\t0\t0\t0",
            "SKATEFLAG\t1\tS\t1\t0\t0\t0\t-1",
            "SKATEFLAG\t1\tR\t0\t0",
            "SKATEFLAG\t1\tE\t1\t256",
            "SKATEFLAG\t1\tF\t0\t0\textra",
        ] {
            assert!(Message::decode(text).is_err(), "accepted {text:?}");
        }
    }
}

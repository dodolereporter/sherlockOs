//! The calendar under the clock: the days of a month in weeks that start on Monday, with the
//! days of the months around it to fill the first and last weeks. Proleptic Gregorian
//! calendar, without the C library: only the clock's date is local.

/// A month of a year. `month` goes from 1 to 12.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Month {
    pub year: i32,
    pub month: u8,
}

/// A day of a month.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

/// A cell of the month grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub date: Date,
    /// The day belongs to the month shown, not to the one before or after.
    pub in_month: bool,
}

/// Whether `year` has a 29 February: every fourth year, but not every hundredth, but every
/// four hundredth.
pub fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

impl Month {
    /// The month of `date`.
    pub fn of(date: Date) -> Self {
        Self {
            year: date.year,
            month: date.month,
        }
    }

    /// Number of days, 28 to 31. A month out of range has none.
    pub fn days(self) -> u8 {
        match self.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if is_leap(self.year) => 29,
            2 => 28,
            _ => 0,
        }
    }

    pub fn next(self) -> Self {
        match self.month {
            12.. => Self {
                year: self.year.saturating_add(1),
                month: 1,
            },
            m => Self {
                year: self.year,
                month: m + 1,
            },
        }
    }

    pub fn previous(self) -> Self {
        match self.month {
            0 | 1 => Self {
                year: self.year.saturating_sub(1),
                month: 12,
            },
            m => Self {
                year: self.year,
                month: m - 1,
            },
        }
    }

    /// The `weeks` weeks shown for this month, Monday first: the week of its first day, then
    /// the following ones. Six weeks hold any month.
    pub fn grid(self, weeks: usize) -> Vec<Cell> {
        let first = Date {
            year: self.year,
            month: self.month,
            day: 1,
        };
        // Days of the previous month in the first week.
        let before = weekday(first) - 1;
        let previous = self.previous();
        let mut cells = Vec::with_capacity(weeks * 7);
        for day in (previous.days() - before + 1)..=previous.days() {
            cells.push(Cell {
                date: previous.on(day),
                in_month: false,
            });
        }
        for day in 1..=self.days() {
            cells.push(Cell {
                date: self.on(day),
                in_month: true,
            });
        }
        let next = self.next();
        let mut day = 1;
        while cells.len() < weeks * 7 {
            cells.push(Cell {
                date: next.on(day),
                in_month: false,
            });
            day += 1;
        }
        cells.truncate(weeks * 7);
        cells
    }

    fn on(self, day: u8) -> Date {
        Date {
            year: self.year,
            month: self.month,
            day,
        }
    }
}

/// Day of the week of `date`, from 1 (Monday) to 7 (Sunday): Sakamoto's method.
pub fn weekday(date: Date) -> u8 {
    const OFFSETS: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let month = usize::from(date.month.clamp(1, 12));
    let mut year = i64::from(date.year);
    if month < 3 {
        year -= 1;
    }
    let sunday_first = (year + year.div_euclid(4) - year.div_euclid(100)
        + year.div_euclid(400)
        + OFFSETS[month - 1]
        + i64::from(date.day))
    .rem_euclid(7);
    // 0 is Sunday: it becomes 7.
    if sunday_first == 0 {
        7
    } else {
        sunday_first as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u8, day: u8) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn leap_years_follow_the_gregorian_rule() {
        assert!(is_leap(2024) && is_leap(2028) && is_leap(2000) && is_leap(2400));
        assert!(!is_leap(2026) && !is_leap(1900) && !is_leap(2100));
        let february = |year| Month { year, month: 2 }.days();
        assert_eq!(
            (february(2028), february(2026), february(2000)),
            (29, 28, 29)
        );
        assert_eq!(february(1900), 28);
        assert_eq!(
            Month {
                year: 2026,
                month: 4
            }
            .days(),
            30
        );
        assert_eq!(
            Month {
                year: 2026,
                month: 12
            }
            .days(),
            31
        );
    }

    #[test]
    fn weekdays_of_known_dates() {
        // The design board's date: Tuesday 24 November 2026.
        assert_eq!(weekday(date(2026, 11, 24)), 2);
        assert_eq!(weekday(date(2026, 11, 1)), 7);
        assert_eq!(weekday(date(2000, 1, 1)), 6);
        assert_eq!(weekday(date(2024, 2, 29)), 4);
        assert_eq!(weekday(date(2028, 3, 1)), 3);
        assert_eq!(weekday(date(1970, 1, 1)), 4);
    }

    #[test]
    fn months_change_across_years() {
        let december = Month {
            year: 2026,
            month: 12,
        };
        assert_eq!(
            december.next(),
            Month {
                year: 2027,
                month: 1
            }
        );
        assert_eq!(december.next().previous(), december);
        assert_eq!(
            Month {
                year: 2026,
                month: 1
            }
            .previous(),
            Month {
                year: 2025,
                month: 12
            }
        );
        let mut month = Month {
            year: 2026,
            month: 3,
        };
        for _ in 0..24 {
            month = month.next();
        }
        assert_eq!(
            month,
            Month {
                year: 2028,
                month: 3
            }
        );
    }

    #[test]
    fn the_grid_starts_on_monday_and_fills_six_weeks() {
        // November 2026 starts on a Sunday: six days of October before it.
        let grid = Month {
            year: 2026,
            month: 11,
        }
        .grid(6);
        assert_eq!(grid.len(), 42);
        assert_eq!(grid[0].date, date(2026, 10, 26));
        assert!(!grid[0].in_month);
        assert_eq!(grid[6].date, date(2026, 11, 1));
        assert!(grid[6].in_month);
        assert_eq!(grid[35].date, date(2026, 11, 30));
        assert_eq!(grid[36].date, date(2026, 12, 1));
        for (i, cell) in grid.iter().enumerate() {
            assert_eq!(usize::from(weekday(cell.date)), i % 7 + 1, "{cell:?}");
        }
        // February 2027 starts on a Monday: no day before it, two weeks after.
        let february = Month {
            year: 2027,
            month: 2,
        }
        .grid(6);
        assert_eq!(february[0].date, date(2027, 2, 1));
        assert_eq!(february[27].date, date(2027, 2, 28));
        assert_eq!(february[28].date, date(2027, 3, 1));
        // A leap February, and December into January.
        let leap = Month {
            year: 2028,
            month: 2,
        }
        .grid(6);
        assert!(leap.iter().any(|c| c.in_month && c.date.day == 29));
        let december = Month {
            year: 2026,
            month: 12,
        }
        .grid(6);
        assert_eq!(december[0].date, date(2026, 11, 30));
        assert_eq!(december[41].date, date(2027, 1, 10));
        assert_eq!(december.iter().filter(|c| c.in_month).count(), 31);
    }
}

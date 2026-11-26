//! The calculator: a query that is an arithmetic expression gives its result, and Enter copies
//! it. `+ - * / ^ %`, parentheses, decimals written with a comma or a point; `×` and `÷` too.
//! `%` is a modulo between two numbers and a percent after one (`200 * 15%`). A hand-written
//! recursive descent parser: no dependency for so small a grammar.

use sherlock_ui::launcher::{Glyph, text};

use crate::source::{Action, Hit, Query, Source, Tile};

/// Longest query read as an expression, and deepest nesting: a query is typed by hand.
const MAX_LENGTH: usize = 200;
const MAX_DEPTH: usize = 32;
/// Significant digits of a result: enough for a desk calculator, few enough to hide the
/// binary noise (`0.1 + 0.2` gives `0.3`).
const DIGITS: i32 = 12;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Number(f64),
    Op(char),
    Open,
    Close,
}

fn tokens(input: &str) -> Option<Vec<Token>> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            '0'..='9' | '.' | ',' => {
                let mut number = String::from(c);
                while let Some(&d) = chars
                    .peek()
                    .filter(|d| d.is_ascii_digit() || **d == '.' || **d == ',')
                {
                    number.push(d);
                    chars.next();
                }
                let number = number.replace(',', ".");
                // One separator at most, and a digit somewhere.
                if number.matches('.').count() > 1 || !number.chars().any(|d| d.is_ascii_digit()) {
                    return None;
                }
                out.push(Token::Number(number.parse().ok()?));
            }
            '+' | '-' | '*' | '/' | '^' | '%' => out.push(Token::Op(c)),
            '×' => out.push(Token::Op('*')),
            '÷' => out.push(Token::Op('/')),
            '(' => out.push(Token::Open),
            ')' => out.push(Token::Close),
            _ => return None,
        }
    }
    Some(out)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
    /// Operations met: a lone number is not a calculation.
    operations: usize,
}

impl Parser {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.at).copied()
    }

    fn eat(&mut self, op: char) -> bool {
        let found = self.peek() == Some(Token::Op(op));
        if found {
            self.at += 1;
        }
        found
    }

    /// Whether the token at `at` can start an operand.
    fn operand_at(&self, at: usize) -> bool {
        matches!(
            self.tokens.get(at),
            Some(Token::Number(_) | Token::Open | Token::Op('-' | '+'))
        )
    }

    fn binary(&mut self, value: f64, op: char, right: f64) -> Option<f64> {
        self.operations += 1;
        let result = match op {
            '+' => value + right,
            '-' => value - right,
            '*' => value * right,
            '/' if right == 0.0 => return None,
            '/' => value / right,
            '%' if right == 0.0 => return None,
            '%' => value % right,
            _ => value.powf(right),
        };
        result.is_finite().then_some(result)
    }

    /// expr := term (('+' | '-') term)*
    fn expr(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            let op = if self.eat('+') {
                '+'
            } else if self.eat('-') {
                '-'
            } else {
                return Some(value);
            };
            let right = self.term()?;
            value = self.binary(value, op, right)?;
        }
    }

    /// term := unary (('*' | '/' | '%') unary)*, `%` being a modulo only before an operand.
    fn term(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Op(op @ ('*' | '/'))) => op,
                Some(Token::Op('%')) if self.operand_at(self.at + 1) => '%',
                _ => return Some(value),
            };
            self.at += 1;
            let right = self.unary()?;
            value = self.binary(value, op, right)?;
        }
    }

    /// unary := ('-' | '+') unary | power. `-2^2` is `-(2^2)`, as written on paper.
    fn unary(&mut self) -> Option<f64> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let value = if self.eat('-') {
            Some(-self.unary()?)
        } else if self.eat('+') {
            self.unary()
        } else {
            self.power()
        };
        self.depth -= 1;
        value
    }

    /// power := percent ('^' unary)?, to the right: `2^3^2` is `2^9`.
    fn power(&mut self) -> Option<f64> {
        let base = self.percent()?;
        if self.eat('^') {
            let exponent = self.unary()?;
            return self.binary(base, '^', exponent);
        }
        Some(base)
    }

    /// percent := primary ('%' not followed by an operand)*
    fn percent(&mut self) -> Option<f64> {
        let mut value = self.primary()?;
        while self.peek() == Some(Token::Op('%')) && !self.operand_at(self.at + 1) {
            self.at += 1;
            self.operations += 1;
            value /= 100.0;
        }
        Some(value)
    }

    /// primary := number | '(' expr ')'
    fn primary(&mut self) -> Option<f64> {
        match self.peek()? {
            Token::Number(n) => {
                self.at += 1;
                Some(n)
            }
            Token::Open => {
                self.at += 1;
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return None;
                }
                let value = self.expr()?;
                if self.peek() != Some(Token::Close) {
                    return None;
                }
                self.at += 1;
                self.depth -= 1;
                Some(value)
            }
            _ => None,
        }
    }
}

/// The value of `input`, when it is a whole expression with at least one operation; `None`
/// for anything else, a division by zero or a result too large.
pub fn evaluate(input: &str) -> Option<f64> {
    if input.len() > MAX_LENGTH {
        return None;
    }
    let mut parser = Parser {
        tokens: tokens(input)?,
        at: 0,
        depth: 0,
        operations: 0,
    };
    let value = parser.expr()?;
    let whole = parser.at == parser.tokens.len();
    (whole && parser.operations > 0 && value.is_finite()).then_some(value + 0.0)
}

/// `value` written for a person: `DIGITS` significant digits, no trailing zeros, the decimal
/// separator given (a comma in French).
pub fn format(value: f64, separator: char) -> String {
    if value == 0.0 {
        return String::from("0");
    }
    let magnitude = value.abs().log10().floor() as i32;
    if !(-6..15).contains(&magnitude) {
        let written = format!("{:.*e}", (DIGITS - 1) as usize, value);
        let (mantissa, exponent) = written.split_once('e').unwrap_or((&written, "0"));
        let mantissa = trim(mantissa);
        return format!("{mantissa}e{exponent}").replace('.', &separator.to_string());
    }
    let decimals = (DIGITS - 1 - magnitude).max(0) as usize;
    let written = trim(&format!("{value:.decimals$}"));
    let written = if written == "-0" {
        String::from("0")
    } else {
        written
    };
    written.replace('.', &separator.to_string())
}

fn trim(number: &str) -> String {
    if number.contains('.') {
        number
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    } else {
        number.to_owned()
    }
}

/// The calculator, a source with one result at most.
#[derive(Debug, Clone, Copy, Default)]
pub struct Calculator;

impl Source for Calculator {
    fn section(&self) -> &'static str {
        text::CALCULATOR
    }

    /// The result, written with a point when the query used one, with a comma otherwise.
    fn search(&self, query: &Query, limit: usize) -> Vec<Hit> {
        let Some(value) = evaluate(&query.raw).filter(|_| limit > 0) else {
            return Vec::new();
        };
        let separator = if query.raw.contains('.') { '.' } else { ',' };
        let result = format(value, separator);
        vec![Hit {
            title: result.clone(),
            subtitle: Some(text::COPY.to_owned()),
            tile: Tile::Glyph(Glyph::Calculator),
            app: None,
            action: Action::Copy(result),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(input: &str) -> Option<f64> {
        evaluate(input)
    }

    #[test]
    fn the_operations_and_their_order() {
        assert_eq!(eval("1 + 2 * 3"), Some(7.0));
        assert_eq!(eval("(1 + 2) * 3"), Some(9.0));
        assert_eq!(eval("10 - 4 - 3"), Some(3.0));
        assert_eq!(eval("2 ^ 3 ^ 2"), Some(512.0));
        assert_eq!(eval("-2^2"), Some(-4.0));
        assert_eq!(eval("2 * -3"), Some(-6.0));
        assert_eq!(eval("7 % 3"), Some(1.0));
        assert_eq!(eval("200 * 15%"), Some(30.0));
        assert_eq!(eval("50%"), Some(0.5));
        assert_eq!(eval("6 × 7 ÷ 2"), Some(21.0));
        assert_eq!(eval("1/4"), Some(0.25));
    }

    #[test]
    fn decimals_with_a_comma_or_a_point() {
        assert_eq!(eval("1,5 + 1.5"), Some(3.0));
        assert_eq!(eval(",5 * 2"), Some(1.0));
        assert_eq!(eval("1,5,2 + 1"), None);
        assert_eq!(eval("1.5.2 + 1"), None);
    }

    #[test]
    fn division_by_zero_and_invalid_expressions_give_nothing() {
        for input in [
            "1 / 0",
            "5 % 0",
            "0 / 0",
            "1 / (2 - 2)",
            "10 ^ 400",
            "2 +",
            "* 3",
            "(1 + 2",
            "1 + 2)",
            "2 3",
            "()",
            "abc",
            "2 + x",
            "",
            "   ",
            ".",
            "1 +* 2",
            "firefox",
        ] {
            assert_eq!(eval(input), None, "{input}");
        }
        // A lone number is not a calculation; a name with digits is no expression either.
        assert_eq!(eval("42"), None);
        assert_eq!(eval("(42)"), None);
        assert_eq!(eval("-42"), None);
        assert_eq!(eval("2048 jeu"), None);
        // Deep nesting and long queries stop early.
        assert_eq!(
            eval(&format!("{}1{}", "(".repeat(100), ")".repeat(100))),
            None
        );
        assert_eq!(eval(&"1+".repeat(150)), None);
        assert_eq!(eval(&"-".repeat(100)), None);
    }

    #[test]
    fn results_are_written_for_a_person() {
        assert_eq!(format(0.1 + 0.2, ','), "0,3");
        assert_eq!(format(3.5, ','), "3,5");
        assert_eq!(format(3.5, '.'), "3.5");
        assert_eq!(format(42.0, ','), "42");
        assert_eq!(format(-0.0, ','), "0");
        assert_eq!(format(1.0 / 3.0, ','), "0,333333333333");
        assert_eq!(format(-1234567.25, ','), "-1234567,25");
        assert_eq!(format(1e20, ','), "1e20");
        assert_eq!(format(2.5e-9, '.'), "2.5e-9");
    }

    #[test]
    fn the_source_copies_the_result() {
        let hits = Calculator.search(&Query::new("1,5 * 3"), 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "4,5");
        assert_eq!(hits[0].action, Action::Copy("4,5".into()));
        let hits = Calculator.search(&Query::new("1.5 * 3"), 5);
        assert_eq!(hits[0].action, Action::Copy("4.5".into()));
        assert!(Calculator.search(&Query::new("1 / 0"), 5).is_empty());
        assert!(Calculator.search(&Query::new("1 + 1"), 0).is_empty());
    }
}

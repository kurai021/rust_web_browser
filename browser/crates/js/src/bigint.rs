//! Bounded signed integers for the language, not a cryptographic implementation.
//! Base-2^32 limbs keep arithmetic portable; work/bit caps bound hostile operands.
use crate::{ErrorKind, JsError};
use std::cmp::Ordering;
const MAX_WORDS: usize = 512;
const MAX_WORK: usize = 4_000_000;
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BigInt {
    negative: bool,
    words: Vec<u32>,
}
fn limit() -> JsError {
    JsError::new(ErrorKind::RangeError, "BigInt bit/work quota")
}
impl BigInt {
    pub fn zero() -> Self {
        Self {
            negative: false,
            words: Vec::new(),
        }
    }
    pub fn one() -> Self {
        Self {
            negative: false,
            words: vec![1],
        }
    }
    pub fn is_zero(&self) -> bool {
        self.words.is_empty()
    }
    pub fn is_negative(&self) -> bool {
        self.negative
    }
    pub fn retained_bytes(&self) -> usize {
        32 + self.words.capacity() * 4
    }
    fn normalized(mut self) -> Self {
        while self.words.last() == Some(&0) {
            self.words.pop();
        }
        if self.words.is_empty() {
            self.negative = false;
        }
        self
    }
    fn bounded(self) -> Result<Self, JsError> {
        if self.words.len() > MAX_WORDS {
            Err(limit())
        } else {
            Ok(self.normalized())
        }
    }
    pub fn parse(source: &str) -> Result<Self, JsError> {
        let mut source = source.trim();
        if source.is_empty() {
            return Ok(Self::zero());
        }
        if source.len() > 65536 {
            return Err(limit());
        }
        let mut negative = false;
        let signed = source.starts_with(['-', '+']);
        if let Some(s) = source.strip_prefix('-') {
            negative = true;
            source = s;
        } else if let Some(s) = source.strip_prefix('+') {
            source = s;
        }
        let (radix, digits) = if source.starts_with("0x") || source.starts_with("0X") {
            (16, &source[2..])
        } else if source.starts_with("0b") || source.starts_with("0B") {
            (2, &source[2..])
        } else if source.starts_with("0o") || source.starts_with("0O") {
            (8, &source[2..])
        } else {
            (10, source)
        };
        if digits.is_empty() || signed && radix != 10 {
            return Err(JsError::new(
                ErrorKind::SyntaxError,
                "invalid BigInt string",
            ));
        }
        let mut out = Self::zero();
        let mut work = 0usize;
        for c in digits.chars() {
            let n = c
                .to_digit(radix)
                .ok_or_else(|| JsError::new(ErrorKind::SyntaxError, "invalid BigInt digit"))?;
            work = work.saturating_add(out.words.len() + 1);
            if work > MAX_WORK {
                return Err(limit());
            }
            out.mul_small(radix)?;
            out.add_small(n)?;
        }
        out.negative = negative;
        Ok(out.normalized())
    }
    pub fn from_f64(value: f64) -> Result<Self, JsError> {
        if !value.is_finite() || value.fract() != 0.0 {
            return Err(JsError::new(
                ErrorKind::RangeError,
                "BigInt requires an integer",
            ));
        }
        if value == 0.0 {
            return Ok(Self::zero());
        }
        let bits = value.abs().to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023;
        let mantissa = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
        let mut out = Self {
            negative: value < 0.0,
            words: vec![mantissa as u32, (mantissa >> 32) as u32],
        }
        .normalized();
        if exponent >= 52 {
            out = out.shift_left((exponent - 52) as usize)?;
        } else {
            out = out.shift_right_unsigned((52 - exponent) as usize);
        }
        Ok(out.normalized())
    }
    pub fn to_f64(&self) -> f64 {
        let value = self
            .words
            .iter()
            .rev()
            .fold(0.0, |n, w| n * 4294967296.0 + *w as f64);
        if self.negative {
            -value
        } else {
            value
        }
    }
    pub fn negated(&self) -> Self {
        let mut out = self.clone();
        if !out.is_zero() {
            out.negative = !out.negative;
        }
        out
    }
    fn cmp_abs(&self, other: &Self) -> Ordering {
        self.words
            .len()
            .cmp(&other.words.len())
            .then_with(|| self.words.iter().rev().cmp(other.words.iter().rev()))
    }
    pub fn compare(&self, other: &Self) -> Ordering {
        match (self.negative, other.negative) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (true, true) => other.cmp_abs(self),
            _ => self.cmp_abs(other),
        }
    }
    pub fn compare_number(&self, value: f64) -> Option<Ordering> {
        if value.is_nan() {
            return None;
        }
        if value == f64::INFINITY {
            return Some(Ordering::Less);
        }
        if value == f64::NEG_INFINITY {
            return Some(Ordering::Greater);
        }
        let floor = value.floor();
        let integer = Self::from_f64(floor).ok()?;
        let cmp = self.compare(&integer);
        Some(if cmp == Ordering::Equal && value != floor {
            Ordering::Less
        } else {
            cmp
        })
    }
    fn add_abs(&self, other: &Self) -> Result<Self, JsError> {
        let mut words = Vec::new();
        let mut carry = 0u64;
        for i in 0..self.words.len().max(other.words.len()) {
            let n = self.words.get(i).copied().unwrap_or(0) as u64
                + other.words.get(i).copied().unwrap_or(0) as u64
                + carry;
            words.push(n as u32);
            carry = n >> 32;
        }
        if carry != 0 {
            words.push(carry as u32);
        }
        Self {
            negative: false,
            words,
        }
        .bounded()
    }
    fn sub_abs(&self, other: &Self) -> Self {
        let mut words = Vec::new();
        let mut borrow = 0i64;
        for (i, &a) in self.words.iter().enumerate() {
            let n = a as i64 - other.words.get(i).copied().unwrap_or(0) as i64 - borrow;
            words.push(n as u32);
            borrow = if n < 0 { 1 } else { 0 };
        }
        Self {
            negative: false,
            words,
        }
        .normalized()
    }
    pub fn add(&self, other: &Self) -> Result<Self, JsError> {
        let mut out = if self.negative == other.negative {
            self.add_abs(other)?
        } else {
            match self.cmp_abs(other) {
                Ordering::Less => other.sub_abs(self),
                _ => self.sub_abs(other),
            }
        };
        out.negative = if self.negative == other.negative {
            self.negative
        } else if self.cmp_abs(other) == Ordering::Less {
            other.negative
        } else {
            self.negative
        };
        Ok(out.normalized())
    }
    pub fn sub(&self, other: &Self) -> Result<Self, JsError> {
        self.add(&other.negated())
    }
    pub fn mul(&self, other: &Self) -> Result<Self, JsError> {
        if self.is_zero() || other.is_zero() {
            return Ok(Self::zero());
        }
        if self.words.len() + other.words.len() > MAX_WORDS + 1
            || self.words.len().saturating_mul(other.words.len()) > MAX_WORK
        {
            return Err(limit());
        }
        let mut words = vec![0u32; self.words.len() + other.words.len()];
        for (i, &a) in self.words.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in other.words.iter().enumerate() {
                let n = a as u64 * b as u64 + words[i + j] as u64 + carry;
                words[i + j] = n as u32;
                carry = n >> 32;
            }
            words[i + other.words.len()] = carry as u32;
        }
        Self {
            negative: self.negative != other.negative,
            words,
        }
        .bounded()
    }
    fn bits(&self) -> usize {
        self.words.last().map_or(0, |w| {
            (self.words.len() - 1) * 32 + (32 - w.leading_zeros() as usize)
        })
    }
    fn shift_left(&self, count: usize) -> Result<Self, JsError> {
        if self.is_zero() {
            return Ok(Self::zero());
        }
        if self.bits().saturating_add(count) > MAX_WORDS * 32 {
            return Err(limit());
        }
        let mut words = vec![0; count / 32];
        let mut carry = 0u64;
        for &word in &self.words {
            let n = ((word as u64) << (count % 32)) | carry;
            words.push(n as u32);
            carry = n >> 32;
        }
        if carry != 0 {
            words.push(carry as u32);
        }
        Self {
            negative: self.negative,
            words,
        }
        .bounded()
    }
    fn shift_right_unsigned(&self, count: usize) -> Self {
        if count >= self.bits() {
            return Self::zero();
        }
        let mut words = self.words[count / 32..].to_vec();
        let bits = count % 32;
        if bits != 0 {
            let mut carry = 0u32;
            for word in words.iter_mut().rev() {
                let old = *word;
                *word = (old >> bits) | carry;
                carry = old << (32 - bits);
            }
        }
        Self {
            negative: self.negative,
            words,
        }
        .normalized()
    }
    fn shift_amount(&self) -> i64 {
        let max = (MAX_WORDS * 32 + 1) as u64;
        let mut count = 0u64;
        for word in self.words.iter().rev() {
            count = count
                .saturating_mul(4294967296)
                .saturating_add(*word as u64)
                .min(max);
        }
        if self.negative {
            -(count as i64)
        } else {
            count as i64
        }
    }
    pub fn shift(&self, amount: &Self, left: bool) -> Result<Self, JsError> {
        let count = amount.shift_amount();
        let left = if count < 0 { !left } else { left };
        let count = count.unsigned_abs() as usize;
        if left {
            return self.shift_left(count);
        }
        let mut out = self.shift_right_unsigned(count);
        if self.negative {
            let lost = self.words.iter().enumerate().any(|(i, &w)| {
                if i < count / 32 {
                    w != 0
                } else if i == count / 32 && !count.is_multiple_of(32) {
                    w & ((1u32 << (count % 32)) - 1) != 0
                } else {
                    false
                }
            });
            if lost {
                out = out.sub(&Self::one())?;
            }
        }
        Ok(out)
    }
    pub fn div_rem(&self, other: &Self) -> Result<(Self, Self), JsError> {
        if other.is_zero() {
            return Err(JsError::new(
                ErrorKind::RangeError,
                "BigInt division by zero",
            ));
        }
        if self.cmp_abs(other) == Ordering::Less {
            return Ok((Self::zero(), self.clone()));
        }
        let difference = self.bits() - other.bits();
        if (difference + 1).saturating_mul(self.words.len()) > MAX_WORK {
            return Err(limit());
        }
        let mut remainder = self.clone();
        remainder.negative = false;
        let mut divisor = other.clone();
        divisor.negative = false;
        divisor = divisor.shift_left(difference)?;
        let mut quotient = Self {
            negative: self.negative != other.negative,
            words: vec![0; difference / 32 + 1],
        };
        for bit in (0..=difference).rev() {
            if remainder.cmp_abs(&divisor) != Ordering::Less {
                remainder = remainder.sub_abs(&divisor);
                quotient.words[bit / 32] |= 1u32 << (bit % 32);
            }
            divisor = divisor.shift_right_unsigned(1);
        }
        remainder.negative = self.negative;
        Ok((quotient.normalized(), remainder.normalized()))
    }
    fn twos(&self, length: usize) -> Vec<u32> {
        let mut out = self.words.clone();
        out.resize(length, 0);
        if self.negative {
            for word in &mut out {
                *word = !*word;
            }
            let mut carry = true;
            for word in &mut out {
                if !carry {
                    break;
                }
                let (n, c) = word.overflowing_add(1);
                *word = n;
                carry = c;
            }
        }
        out
    }
    pub fn bitwise(&self, other: &Self, op: &str) -> Result<Self, JsError> {
        let length = self.words.len().max(other.words.len()) + 1;
        let a = self.twos(length);
        let b = other.twos(length);
        let mut words = a
            .into_iter()
            .zip(b)
            .map(|(a, b)| match op {
                "&" => a & b,
                "|" => a | b,
                _ => a ^ b,
            })
            .collect::<Vec<_>>();
        let negative = words.last().is_some_and(|w| w & (1 << 31) != 0);
        if negative {
            for word in &mut words {
                *word = !*word;
            }
            let mut carry = true;
            for word in &mut words {
                if !carry {
                    break;
                }
                let (n, c) = word.overflowing_add(1);
                *word = n;
                carry = c;
            }
        }
        Self { negative, words }.bounded()
    }
    pub fn not(&self) -> Result<Self, JsError> {
        self.negated().sub(&Self::one())
    }
    pub fn pow(&self, exponent: &Self) -> Result<Self, JsError> {
        if exponent.negative {
            return Err(JsError::new(
                ErrorKind::RangeError,
                "negative BigInt exponent",
            ));
        }
        let n = exponent.shift_amount() as usize;
        if n > MAX_WORDS * 32 {
            return Err(limit());
        }
        let mut out = Self::one();
        let mut base = self.clone();
        let mut n = n;
        while n != 0 {
            if n & 1 != 0 {
                out = out.mul(&base)?;
            }
            n >>= 1;
            if n != 0 {
                base = base.mul(&base)?;
            }
        }
        Ok(out)
    }
    pub fn as_bits(&self, bits: usize, signed: bool) -> Result<Self, JsError> {
        if bits > MAX_WORDS * 32 {
            return Err(limit());
        }
        if bits == 0 {
            return Ok(Self::zero());
        }
        let mut words = self.twos(bits.div_ceil(32));
        words.truncate(bits.div_ceil(32));
        if !bits.is_multiple_of(32) {
            if let Some(word) = words.last_mut() {
                *word &= (1u32 << (bits % 32)) - 1;
            }
        }
        let out = Self {
            negative: false,
            words,
        }
        .normalized();
        if signed
            && out
                .words
                .get((bits - 1) / 32)
                .is_some_and(|w| w & (1u32 << ((bits - 1) % 32)) != 0)
        {
            let mut power = Self::one().shift_left(bits - 1)?;
            power = power.add(&power)?;
            let mut result = power.sub_abs(&out);
            result.negative = true;
            Ok(result.normalized())
        } else {
            Ok(out)
        }
    }
    fn mul_small(&mut self, value: u32) -> Result<(), JsError> {
        let mut carry = 0u64;
        for word in &mut self.words {
            let n = *word as u64 * value as u64 + carry;
            *word = n as u32;
            carry = n >> 32;
        }
        if carry != 0 {
            self.words.push(carry as u32);
        }
        if self.words.len() > MAX_WORDS {
            return Err(limit());
        }
        Ok(())
    }
    fn add_small(&mut self, value: u32) -> Result<(), JsError> {
        let mut carry = value as u64;
        for word in &mut self.words {
            if carry == 0 {
                break;
            }
            let n = *word as u64 + carry;
            *word = n as u32;
            carry = n >> 32;
        }
        if carry != 0 {
            self.words.push(carry as u32);
        }
        if self.words.len() > MAX_WORDS {
            return Err(limit());
        }
        Ok(())
    }
    fn div_small(&mut self, value: u32) -> u32 {
        let mut remainder = 0u64;
        for word in self.words.iter_mut().rev() {
            let n = (remainder << 32) | *word as u64;
            *word = (n / value as u64) as u32;
            remainder = n % value as u64;
        }
        while self.words.last() == Some(&0) {
            self.words.pop();
        }
        remainder as u32
    }
    pub fn to_radix(&self, radix: u32) -> Result<String, JsError> {
        if !(2..=36).contains(&radix) {
            return Err(JsError::new(ErrorKind::RangeError, "BigInt radix"));
        }
        if self.is_zero() {
            return Ok("0".into());
        }
        let mut n = self.clone();
        n.negative = false;
        let mut chars = Vec::new();
        while !n.is_zero() {
            let digit = n.div_small(radix);
            chars.push(char::from_digit(digit, radix).ok_or_else(limit)?);
        }
        if self.negative {
            chars.push('-');
        }
        Ok(chars.into_iter().rev().collect())
    }
}

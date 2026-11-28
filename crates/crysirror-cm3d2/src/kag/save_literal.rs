// SPDX-License-Identifier: MPL-2.0
//! Native Dictionary.toStructString uses TJS literal tokens. Decode the
//! emitted values directly so reading a slot cannot execute the expression.
use crate::tjs::Host;
use anyhow::{Context, Result, bail, ensure};
use crysirror_tjs2::{
    frontend::lexer::{Token, TokenKind as K, lex},
    runtime::{Runtime, Variant},
};
use std::iter::Peekable;

pub(super) fn read(runtime: &mut Runtime<Host>, text: &str) -> Result<Variant> {
    ensure!(
        text.len() <= 1024 * 1024,
        "KAG checkpoint literal too large"
    );
    let tokens = lex(text).context("KAG checkpoint literal tokens")?;
    let mut parser = Literal {
        tokens: tokens.into_iter().peekable(),
        runtime,
    };
    let value = parser.value(0)?;
    parser.expect(K::Eof)?;
    Ok(value)
}

struct Literal<'a, I: Iterator<Item = Token>> {
    tokens: Peekable<I>,
    runtime: &'a mut Runtime<Host>,
}

impl<I: Iterator<Item = Token>> Literal<'_, I> {
    fn next(&mut self) -> Result<K> {
        Ok(self
            .tokens
            .next()
            .context("incomplete checkpoint literal")?
            .kind)
    }

    fn eat(&mut self, wanted: &K) -> bool {
        self.tokens.next_if(|t| &t.kind == wanted).is_some()
    }

    fn expect(&mut self, wanted: K) -> Result<()> {
        ensure!(
            self.eat(&wanted),
            "invalid checkpoint literal, expected {wanted:?}"
        );
        Ok(())
    }

    fn value(&mut self, depth: usize) -> Result<Variant> {
        ensure!(depth <= 32, "KAG checkpoint literal nesting");
        Ok(match self.next()? {
            K::KwVoid => Variant::Void,
            K::KwNull => Variant::Null,
            K::KwTrue => Variant::Integer(1),
            K::KwFalse => Variant::Integer(0),
            K::String(value) => Variant::String(value),
            K::Octet(value) => Variant::Octet(value),
            K::Integer(value) => Variant::Integer(value),
            K::Real(value) => Variant::Real(value),
            K::Minus => match self.next()? {
                K::Integer(value) => {
                    Variant::Integer(value.checked_neg().context("integer overflow")?)
                }
                K::Real(value) => Variant::Real(-value),
                _ => bail!("invalid negative checkpoint literal"),
            },
            K::LeftParen => {
                self.expect(K::KwConst)?;
                self.expect(K::RightParen)?;
                self.collection(depth + 1)?
            }
            _ => bail!("nonliteral expression in KAG checkpoint"),
        })
    }

    fn collection(&mut self, depth: usize) -> Result<Variant> {
        let dictionary = self.eat(&K::Percent);
        self.expect(K::LeftBracket)?;
        if dictionary {
            let object = self.runtime.alloc_dictionary_object();
            if !self.eat(&K::RightBracket) {
                loop {
                    let K::String(key) = self.next()? else {
                        bail!("non-string KAG checkpoint dictionary key");
                    };
                    self.expect(K::Comma)?;
                    let value = self.value(depth)?;
                    ensure!(
                        !self.runtime.has_object_member(object, &key),
                        "duplicate KAG checkpoint dictionary key"
                    );
                    self.runtime.set_object_member(object, key, value);
                    if self.eat(&K::RightBracket) {
                        break;
                    }
                    self.expect(K::Comma)?;
                }
            }
            Ok(Variant::Object(object))
        } else {
            let mut values = Vec::new();
            if !self.eat(&K::RightBracket) {
                loop {
                    values.push(self.value(depth)?);
                    if self.eat(&K::RightBracket) {
                        break;
                    }
                    self.expect(K::Comma)?;
                }
            }
            Ok(Variant::Object(self.runtime.alloc_array_object(values)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_accepts_native_literals_and_rejects_tjs_execution() -> Result<()> {
        let mut tjs = crate::tjs::Tjs::with_state(
            Default::default(),
            crate::tjs::ScriptProfile::Com3d2,
            Default::default(),
        )?;
        let root = read(
            &mut tjs.runtime,
            r#"(const)%["storageName","main.ks","callStack",(const)[(const)%["offset",-7]],"seed",<%01 ff%>]"#,
        )?.object_handle().unwrap();
        assert_eq!(
            tjs.runtime.object_member(root, "storageName"),
            Variant::String("main.ks".into())
        );
        assert_eq!(
            tjs.runtime.object_member(root, "seed"),
            Variant::Octet(vec![1, 255])
        );
        let frames = tjs
            .runtime
            .object_member(root, "callStack")
            .object_handle()
            .unwrap();
        let frame = tjs.runtime.array_elements(frames).unwrap()[0]
            .object_handle()
            .unwrap();
        assert_eq!(
            tjs.runtime.object_member(frame, "offset"),
            Variant::Integer(-7)
        );
        for bad in [
            r#"(const)%["callStack",(global.saved = 1)]"#,
            r#"(const)%["player",SetPlayerData("所持金", 0)]"#,
            r#"(const)%["x",0];global.saved = 1"#,
        ] {
            assert!(read(&mut tjs.runtime, bad).is_err());
        }
        assert_eq!(tjs.runtime.global_member("saved"), Variant::Void);
        Ok(())
    }
}

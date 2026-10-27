#![forbid(unsafe_code)]

//! The edi path technology — a technology of `xmip-core-path`.
//!
//! [`EdiLanguage`] is the [`PathLanguage`] `edi`: a segment and an element
//! of an EDIFACT or X12 interchange, read for promote, route and process
//! and written for demote. The interchange is cut into segments by the
//! contract technologies' own syntax (`xmip-core-contract-edi-edifact` and
//! `-edi-x12`), once per Message however many paths read it; an X12
//! interchange is told by its `ISA`.
//!
//! A path is `TAG[+QUALIFIER][#N]/ELEMENT[/COMPONENT]`, every number from 1:
//! `BGM/2` is the second element of the first `BGM`; `NAD+BY/2/1` the first
//! component of the second element of the `NAD` whose first element is
//! `BY`; `REF#2/2` the second element of the second `REF`. A write
//! replaces one component and writes the interchange again in its own
//! separators, a new Stream (ADR-0013).

use contract::ContractError;
use contract::segment::Segment;
use edifact::syntax::Interchange as Edifact;
use path::{CompiledExpression, Content, Editable, Form, PathLanguage, Rewriting};
use stream::Stream;
use x12::syntax::Interchange as X12;
use xcore::ScalarValue;

/// The language `edi`.
pub struct EdiLanguage;

impl PathLanguage for EdiLanguage {
    fn language(&self) -> &'static str {
        "edi"
    }

    fn compile(&self, expression: &str) -> Result<Box<dyn CompiledExpression>, ContractError> {
        Ok(Box::new(Address::parse(expression)?))
    }
}

/// Where a path points: the segment, which of them, and the component.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Address {
    tag: String,
    qualifier: Option<String>,
    occurrence: usize,
    element: usize,
    component: usize,
}

impl Address {
    fn parse(expression: &str) -> Result<Self, ContractError> {
        let refuse = || {
            ContractError::new(format!(
                "{expression:?} is not TAG[+QUALIFIER][#N]/ELEMENT[/COMPONENT], counted from 1"
            ))
        };
        let mut parts = expression.trim().split('/');
        let head = parts.next().ok_or_else(refuse)?;
        let number = |text: Option<&str>, default: usize| -> Result<usize, ContractError> {
            match text {
                None => Ok(default),
                Some(text) => text
                    .trim()
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(refuse),
            }
        };
        let element = number(Some(parts.next().ok_or_else(refuse)?), 0)?;
        let component = number(parts.next(), 1)?;
        if parts.next().is_some() {
            return Err(refuse());
        }
        let (head, occurrence) = match head.split_once('#') {
            Some((head, n)) => (head, number(Some(n), 1)?),
            None => (head, 1),
        };
        let (tag, qualifier) = match head.split_once('+') {
            Some((tag, qualifier)) => (tag, Some(qualifier.to_string())),
            None => (head, None),
        };
        let sound = (2..=3).contains(&tag.len())
            && tag
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        if !sound {
            return Err(refuse());
        }
        Ok(Self {
            tag: tag.to_string(),
            qualifier,
            occurrence,
            element,
            component,
        })
    }

    /// The index of the segment this path points at, among `segments`.
    fn segment(&self, segments: &[Segment]) -> Option<usize> {
        segments
            .iter()
            .enumerate()
            .filter(|(_, segment)| {
                segment.tag == self.tag
                    && self
                        .qualifier
                        .as_ref()
                        .is_none_or(|qualifier| segment.simple(1) == qualifier)
            })
            .nth(self.occurrence - 1)
            .map(|(at, _)| at)
    }
}

/// An interchange, cut into segments in its own syntax.
enum Interchange {
    Edifact(Edifact),
    X12(X12),
}

impl Interchange {
    fn read(text: &str) -> Result<Self, ContractError> {
        let refused = |issue: contract::ValidationIssue| {
            ContractError::new(format!("not an EDI interchange: {}", issue.message))
        };
        if text.trim_start().starts_with("ISA") {
            X12::parse(text).map(Self::X12).map_err(refused)
        } else {
            Edifact::parse(text).map(Self::Edifact).map_err(refused)
        }
    }

    fn segments(&self) -> &[Segment] {
        match self {
            Self::Edifact(read) => read.segments(),
            Self::X12(read) => read.segments(),
        }
    }

    fn segments_mut(&mut self) -> &mut [Segment] {
        match self {
            Self::Edifact(read) => read.segments_mut(),
            Self::X12(read) => read.segments_mut(),
        }
    }

    fn written(&self) -> Result<String, ContractError> {
        match self {
            Self::Edifact(read) => Ok(read.written()),
            Self::X12(read) => read.written().map_err(ContractError::new),
        }
    }
}

impl Form for Interchange {
    fn parse(stream: &Stream) -> Result<Self, ContractError> {
        let text = stream
            .text()
            .map_err(|error| ContractError::new(format!("not UTF-8 text: {error}")))?;
        Self::read(text)
    }
}

impl Editable for Interchange {
    const MEDIA_TYPE: &'static str = "application/edifact";

    fn open(bytes: &[u8]) -> Result<Self, ContractError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|error| ContractError::new(format!("not UTF-8 text: {error}")))?;
        Self::read(text)
    }

    fn into_bytes(self) -> Result<Vec<u8>, ContractError> {
        self.written().map(String::into_bytes)
    }
}

impl CompiledExpression for Address {
    fn read(&self, content: &Content<'_>) -> Result<Option<ScalarValue>, ContractError> {
        let interchange = content.form::<Interchange>()?;
        let segments = interchange.segments();
        Ok(self
            .segment(segments)
            .and_then(|at| segments[at].element(self.element).get(self.component - 1))
            .map(|found| ScalarValue::Text(found.clone())))
    }

    /// Replace one component. A segment the interchange does not hold is
    /// refused; an element or a component past the end is added, padded
    /// with empty ones, as a segment has no declared width to break.
    fn write(&self, rewriting: &mut Rewriting, value: ScalarValue) -> Result<(), ContractError> {
        let written = match value {
            ScalarValue::Null => String::new(),
            other => other
                .text()
                .ok_or_else(|| ContractError::new("binary has no text form in EDI"))?
                .into_owned(),
        };
        let interchange = rewriting.form_mut::<Interchange>()?;
        let at = self
            .segment(interchange.segments())
            .ok_or_else(|| ContractError::new(format!("the interchange holds no {}", self.tag)))?;
        let segment = &mut interchange.segments_mut()[at];
        if segment.elements.len() < self.element {
            segment.elements.resize(self.element, vec![String::new()]);
        }
        let element = &mut segment.elements[self.element - 1];
        if element.len() < self.component {
            element.resize(self.component, String::new());
        }
        element[self.component - 1] = written;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contract::fixture::stream;
    use xcore::StreamId;

    const ORDERS: &str = "UNA:+.? 'UNB+UNOC:3+SENDER+RECEIVER+260907:1345+REF001'\
UNH+1+ORDERS:D:96A:UN'BGM+220+PO4711'DTM+137:20260907:102'NAD+BY+BUYER'\
NAD+SU+SUPPLIER'UNT+6+1'UNZ+1+REF001'";

    fn read(path: &str, text: &str) -> Option<ScalarValue> {
        let stream = stream(text);
        EdiLanguage
            .compile(path)
            .expect("compiles")
            .read(&Content::of(&stream))
            .expect("reads")
    }

    fn text(value: &str) -> ScalarValue {
        ScalarValue::Text(value.into())
    }

    #[test]
    fn a_segment_element_and_component_are_read_by_tag_qualifier_and_occurrence() {
        assert_eq!(read("BGM/2", ORDERS), Some(text("PO4711")));
        assert_eq!(read("DTM/1/2", ORDERS), Some(text("20260907")));
        assert_eq!(read("NAD+SU/2", ORDERS), Some(text("SUPPLIER")));
        assert_eq!(read("NAD#2/2", ORDERS), Some(text("SUPPLIER")));
        assert_eq!(read("NAD+XX/2", ORDERS), None);
        assert_eq!(read("BGM/9", ORDERS), None);
        for wrong in ["bgm/2", "BGM", "BGM/0", "BGM/a", "BGM/1/2/3", "TOOLONG/1"] {
            assert!(EdiLanguage.compile(wrong).is_err(), "{wrong}");
        }
    }

    const PURCHASE: &str = "ISA*00*          *00*          *ZZ*SENDER         *ZZ*RECEIVER       \
*260907*1345*U*00401*000000001*0*P*:~GS*PO*SENDER*RECEIVER*20260907*1345*1*X*004010~\
ST*850*0001~BEG*00*SA*PO4711**20260907~SE*3*0001~GE*1*1~IEA*1*000000001~";

    #[test]
    fn an_x12_interchange_is_told_by_its_isa_and_written_back_whole() {
        assert_eq!(read("ST/1", PURCHASE), Some(text("850")));
        assert_eq!(read("BEG/3", PURCHASE), Some(text("PO4711")));
        let mut rewriting = Rewriting::of(&stream(PURCHASE), StreamId::new(2));
        EdiLanguage
            .compile("BEG/3")
            .and_then(|compiled| compiled.write(&mut rewriting, ScalarValue::Text("PO4712".into())))
            .expect("writes");
        let written = rewriting.finish().expect("finishes");
        assert_eq!(
            String::from_utf8_lossy(written.bytes()),
            PURCHASE.replace("PO4711", "PO4712")
        );
        let mut rewriting = Rewriting::of(&stream(PURCHASE), StreamId::new(3));
        EdiLanguage
            .compile("BEG/3")
            .and_then(|compiled| compiled.write(&mut rewriting, ScalarValue::Text("a*b".into())))
            .expect("held until written");
        assert!(
            rewriting.finish().is_err(),
            "a separator in a value is refused"
        );
    }

    #[test]
    fn a_component_is_rewritten_into_a_new_stream_released_as_it_must_be() {
        let mut rewriting = Rewriting::of(&stream(ORDERS), StreamId::new(2));
        let write = |path: &str, value: &str, rewriting: &mut Rewriting| {
            EdiLanguage
                .compile(path)
                .and_then(|compiled| compiled.write(rewriting, ScalarValue::Text(value.into())))
        };
        write("BGM/2", "PO+1", &mut rewriting).expect("writes");
        write("NAD+BY/4/2", "late", &mut rewriting).expect("pads");
        assert!(write("QTY/1", "1", &mut rewriting).is_err());
        let written = rewriting.finish().expect("finishes");
        let text = String::from_utf8_lossy(written.bytes()).into_owned();
        assert!(text.contains("BGM+220+PO?+1'"), "{text}");
        assert!(text.contains("NAD+BY+BUYER++:late'"), "{text}");
        assert!(text.starts_with("UNA:+.? 'UNB"), "{text}");
    }
}

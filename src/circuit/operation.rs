#![allow(dead_code)]

use lazy_static::lazy_static;
use symengine::Expression;
pub(crate) type Param = Expression;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum WireType {
    Quantum,
    Classical,
    Bool,
}
#[derive(Clone)]
pub enum Signature {
    Linear(Vec<WireType>),
    NonLinear(Vec<WireType>, Vec<WireType>),
}
pub trait Op {
    fn signature(&self) -> Signature;

    fn get_params(&self) -> Vec<Param>;
}

pub enum GateOp {
    H,
    CX,
    ZZMax,
    Reset,
    Input,
    Output,
    Rx(Param),
    Ry(Param),
    Rz(Param),
    ZZPhase(Param, Param),
    Measure,
}

impl GateOp {
    fn is_one_qb_gate(&self) -> bool {
        match self.signature() {
            Signature::Linear(v) => matches!(&v[..], &[WireType::Quantum]),
            _ => false,
        }
    }

    fn is_two_qb_gate(&self) -> bool {
        match self.signature() {
            Signature::Linear(v) => matches!(&v[..], &[WireType::Quantum, WireType::Quantum]),
            _ => false,
        }
    }
}

lazy_static! {
    static ref ONEQBSIG: Signature = Signature::Linear(vec![WireType::Quantum]);
}
lazy_static! {
    static ref TWOQBSIG: Signature = Signature::Linear(vec![WireType::Quantum, WireType::Quantum]);
}

impl Op for GateOp {
    fn signature(&self) -> Signature {
        match self {
            GateOp::H | GateOp::Reset | GateOp::Rx(_) | GateOp::Ry(_) | GateOp::Rz(_) => {
                ONEQBSIG.clone()
            }
            GateOp::CX | GateOp::ZZMax | GateOp::ZZPhase(..) => TWOQBSIG.clone(),
            _ => panic!("Gate signature unknwon."),
        }
    }

    fn get_params(&self) -> Vec<Param> {
        todo!()
    }
}

pub enum MetaOp {
    Barrier,
}

pub enum ClassicalOp {
    And,
    Xor,
    Or,
}

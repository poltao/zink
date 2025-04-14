use crate::{asm, primitives::Bytes32};

/// Zink event interface
pub trait Event {
    const NAME: &'static [u8];

    fn log0(&self) {
        unsafe {
            asm::evm::log0(Self::NAME);
        }
    }

    fn log1(&self, topic: impl Into<Bytes32>) {
        unsafe { asm::evm::log1(topic.into(), Self::NAME) }
    }

    fn log2(&self, topic1: impl Into<Bytes32>, topic2: impl Into<Bytes32>) {
        unsafe { asm::evm::log2(topic1.into(), topic2.into(), Self::NAME) }
    }

    fn log3(
        &self,
        topic1: impl Into<Bytes32>,
        topic2: impl Into<Bytes32>,
        topic3: impl Into<Bytes32>,
    ) {
        unsafe { asm::evm::log3(topic1.into(), topic2.into(), topic3.into(), Self::NAME) }
    }

    fn log4(
        &self,
        topic1: impl Into<Bytes32>,
        topic2: impl Into<Bytes32>,
        topic3: impl Into<Bytes32>,
        topic4: impl Into<Bytes32>,
    ) {
        unsafe {
            asm::evm::log4(
                topic1.into(),
                topic2.into(),
                topic3.into(),
                topic4.into(),
                Self::NAME,
            )
        }
    }
}

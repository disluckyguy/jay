use {
    crate::{
        client::{Client, ClientError},
        ifs::wl_surface::WlSurface,
        leaks::Tracker,
        object::Object,
        utils::buffd::{MsgParser, MsgParserError},
        wire::{wp_fractional_scale_v1::*, WpFractionalScaleV1Id},
    },
    std::rc::Rc,
    thiserror::Error,
};

pub struct WpFractionalScaleV1 {
    pub id: WpFractionalScaleV1Id,
    pub client: Rc<Client>,
    pub surface: Rc<WlSurface>,
    pub tracker: Tracker<Self>,
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum RoundingAlgorithm {
    PositionDependent,
    PositionIndependent,
}

const ROUND_POSITION_INDEPENDENT: u32 = 0;
const ROUND_POSITION_DEPENDENT: u32 = 1;

impl WpFractionalScaleV1 {
    pub fn new(id: WpFractionalScaleV1Id, surface: &Rc<WlSurface>) -> Self {
        Self {
            id,
            client: surface.client.clone(),
            surface: surface.clone(),
            tracker: Default::default(),
        }
    }

    pub fn install(self: &Rc<Self>) -> Result<(), WpFractionalScaleError> {
        if self.surface.fractional_scale.get().is_some() {
            return Err(WpFractionalScaleError::Exists);
        }
        self.surface.fractional_scale.set(Some(self.clone()));
        Ok(())
    }

    pub fn send_preferred_scale(&self) {
        self.client.event(PreferredScale {
            self_id: self.id,
            scale: self.surface.output.get().preferred_scale.get(),
        });
    }

    fn set_rounding_algorithm(&self, msg: MsgParser<'_, '_>) -> Result<(), WpFractionalScaleError> {
        let req: SetRoundingAlgorithm = self.client.parse(self, msg)?;
        let algorithm = match req.algorithm {
            ROUND_POSITION_INDEPENDENT => RoundingAlgorithm::PositionIndependent,
            ROUND_POSITION_DEPENDENT => RoundingAlgorithm::PositionDependent,
            _ => {
                return Err(WpFractionalScaleError::UnknownRoundingAlgorithm(
                    req.algorithm,
                ))
            }
        };
        self.surface.pending.rounding_algorithm.set(Some(algorithm));
        Ok(())
    }

    fn destroy(&self, msg: MsgParser<'_, '_>) -> Result<(), WpFractionalScaleError> {
        let _req: Destroy = self.client.parse(self, msg)?;
        self.surface.fractional_scale.take();
        self.client.remove_obj(self)?;
        Ok(())
    }
}

object_base! {
    WpFractionalScaleV1;

    DESTROY => destroy,
    SET_ROUNDING_ALGORITHM => set_rounding_algorithm,
}

impl Object for WpFractionalScaleV1 {
    fn num_requests(&self) -> u32 {
        SET_ROUNDING_ALGORITHM + 1
    }
}

simple_add_obj!(WpFractionalScaleV1);

#[derive(Debug, Error)]
pub enum WpFractionalScaleError {
    #[error("Parsing failed")]
    MsgParserError(#[source] Box<MsgParserError>),
    #[error(transparent)]
    ClientError(Box<ClientError>),
    #[error("The surface already has a fractional scale extension attached")]
    Exists,
    #[error("Unknown rounding algorithm {0}")]
    UnknownRoundingAlgorithm(u32),
}
efrom!(WpFractionalScaleError, MsgParserError);
efrom!(WpFractionalScaleError, ClientError);

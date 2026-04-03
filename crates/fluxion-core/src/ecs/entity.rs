// ============================================================
// fluxion-core — Entity identifier
//
// An Entity is just an opaque integer handle — it has no data itself.
// Data lives in Components attached to the entity.
//
// We wrap hecs::Entity in a newtype so:
//   1. The public API doesn't expose hecs directly (we can swap it later).
//   2. The type name "EntityId" is clearer to C++/C# developers than
//      the raw hecs handle.
//
// C++ equivalent:  typedef uint32_t EntityId;
// C#  equivalent:  readonly struct EntityId { public readonly int Value; }
// ============================================================

use serde::{Deserialize, Serialize, Serializer, Deserializer};

/// Opaque entity handle. Cheap to copy (it's just an integer internally).
///
/// Do NOT construct this yourself — always use `ECSWorld::spawn()`.
/// An entity that has been despawned becomes invalid; using it afterwards
/// will return `None` from component lookups (no use-after-free).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityId(pub(crate) hecs::Entity);

// Serialize as the raw u64 bits so scene files remain human-readable.
impl Serialize for EntityId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(self.0.to_bits().into())
    }
}

impl<'de> Deserialize<'de> for EntityId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let bits = u64::deserialize(d)?;
        let entity = hecs::Entity::from_bits(bits)
            .ok_or_else(|| serde::de::Error::custom("invalid EntityId bits"))?;
        Ok(EntityId(entity))
    }
}

impl EntityId {
    /// A guaranteed-invalid sentinel value. Useful as a "no entity" default.
    /// Equivalent to `null` in C# or `entt::null` in EnTT.
    pub const INVALID: EntityId = EntityId(hecs::Entity::DANGLING);

    /// Returns `false` if this is the `INVALID` sentinel.
    /// Note: a non-INVALID EntityId can still be "dead" if its entity was despawned.
    /// Use `ECSWorld::is_alive()` for a definitive liveness check.
    #[inline]
    pub fn is_valid(self) -> bool {
        self != EntityId::INVALID
    }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Entity({:?})", self.0)
    }
}

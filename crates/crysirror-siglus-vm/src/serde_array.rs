use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};
pub fn serialize<S: Serializer, T: Serialize, const N: usize>(
    v: &[T; N],
    s: S,
) -> Result<S::Ok, S::Error> {
    v.as_slice().serialize(s)
}
pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
    d: D,
) -> Result<[T; N], D::Error> {
    let v = Vec::<T>::deserialize(d)?;
    v.try_into()
        .map_err(|_| D::Error::custom("input array length"))
}

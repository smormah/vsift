//! The plain float tensors the runner hands its inference backend and gets back.

use std::fmt;

/// A dense `f32` tensor in row-major order.
#[derive(Clone, Debug, PartialEq)]
pub struct Tensor {
    shape: Vec<usize>,
    data: Vec<f32>,
}

/// A tensor whose data does not fill its shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShapeError;

impl fmt::Display for ShapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the tensor's data does not match its shape")
    }
}

impl std::error::Error for ShapeError {}

impl Tensor {
    /// A tensor of `shape` holding `data`.
    ///
    /// # Errors
    ///
    /// [`ShapeError`] when `data` has not exactly the product of `shape` elements.
    pub fn new(shape: Vec<usize>, data: Vec<f32>) -> Result<Self, ShapeError> {
        let elements = shape
            .iter()
            .try_fold(1_usize, |product, side| product.checked_mul(*side))
            .ok_or(ShapeError)?;
        if shape.is_empty() || elements != data.len() {
            return Err(ShapeError);
        }
        Ok(Self { shape, data })
    }

    /// The sides, outermost first.
    #[must_use]
    pub fn shape(&self) -> &[usize] {
        &self.shape
    }

    /// The elements, row-major.
    #[must_use]
    pub fn data(&self) -> &[f32] {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::Tensor;

    #[test]
    fn a_tensor_must_fill_its_shape() {
        assert!(Tensor::new(vec![2, 3], vec![0.0; 6]).is_ok());
        assert!(Tensor::new(vec![2, 3], vec![0.0; 5]).is_err());
        assert!(Tensor::new(Vec::new(), Vec::new()).is_err());
        assert!(Tensor::new(vec![usize::MAX, 2], Vec::new()).is_err());
    }
}

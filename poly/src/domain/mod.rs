//! This module contains an `EvaluationDomain` abstraction for
//! performing various kinds of polynomial arithmetic on top of
//! fields that are friendly to fast-fourier-transforms (FFTs).
//!
//! A field is FFT-friendly if it contains enough
//! roots of unity to perform the FFT in O(n log n) time.
//! These roots of unity comprise the domain over which
//! polynomial arithmetic is performed.

use ark_ff::FftField;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::{fmt, hash, rand::Rng, vec::Vec};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

pub mod general;
pub mod mixed_radix;
pub mod radix2;
pub(crate) mod utils;

pub use general::GeneralEvaluationDomain;
pub use mixed_radix::MixedRadixEvaluationDomain;
pub use radix2::Radix2EvaluationDomain;

/// Defines a domain over which finite field (I)FFTs can be performed. The
/// size of the supported FFT depends on the size of the multiplicative
/// subgroup. For efficiency, we recommend that the field has at least one large
/// subgroup generated by a root of unity.
pub trait EvaluationDomain<F: FftField>:
    Copy + Clone + hash::Hash + Eq + PartialEq + fmt::Debug + CanonicalSerialize + CanonicalDeserialize
{
    /// The type of the elements iterator.
    type Elements: Iterator<Item = F> + Sized;

    /// Sample an element that is *not* in the domain.
    fn sample_element_outside_domain<R: Rng>(&self, rng: &mut R) -> F {
        let mut t = F::rand(rng);
        while self.evaluate_vanishing_polynomial(t).is_zero() {
            t = F::rand(rng);
        }
        t
    }

    /// Construct a domain that is large enough for evaluations of a polynomial
    /// having `num_coeffs` coefficients.
    fn new(num_coeffs: usize) -> Option<Self>;

    /// Construct a coset domain that is large enough for evaluations of a polynomial
    /// having `num_coeffs` coefficients.
    fn new_coset(num_coeffs: usize, offset: F) -> Option<Self> {
        Self::new(num_coeffs)?.get_coset(offset)
    }

    /// Construct a coset domain from a subgroup domain
    fn get_coset(&self, offset: F) -> Option<Self>;

    /// Return the size of a domain that is large enough for evaluations of a
    /// polynomial having `num_coeffs` coefficients.
    fn compute_size_of_domain(num_coeffs: usize) -> Option<usize>;

    /// Return the size of `self`.
    fn size(&self) -> usize;

    /// Return the size of `self` as a field element.
    fn size_as_field_element(&self) -> F {
        F::from(self.size() as u64)
    }

    /// Return log_2(size) of `self`.
    fn log_size_of_group(&self) -> u64;

    /// Return the inverse of `self.size_as_field_element()`.
    fn size_inv(&self) -> F;

    /// Return the generator for the multiplicative subgroup that defines this domain.
    fn group_gen(&self) -> F;

    /// Return the group inverse of `self.group_gen()`.
    fn group_gen_inv(&self) -> F;

    /// Return the group offset that defines this domain.
    fn coset_offset(&self) -> F;

    /// Return the inverse of `self.offset()`.
    fn coset_offset_inv(&self) -> F;

    /// Return `offset^size`.
    fn coset_offset_pow_size(&self) -> F;

    /// Compute a FFT.
    #[cfg_attr(not(feature = "bin-opt"), inline)]
    fn fft<T: DomainCoeff<F>>(&self, coeffs: &[T]) -> Vec<T> {
        let mut coeffs = coeffs.to_vec();
        self.fft_in_place(&mut coeffs);
        coeffs
    }

    /// Compute a FFT, modifying the vector in place.
    fn fft_in_place<T: DomainCoeff<F>>(&self, coeffs: &mut Vec<T>);

    /// Compute a IFFT.
    #[cfg_attr(not(feature = "bin-opt"), inline)]
    fn ifft<T: DomainCoeff<F>>(&self, evals: &[T]) -> Vec<T> {
        let mut evals = evals.to_vec();
        self.ifft_in_place(&mut evals);
        evals
    }

    /// Compute a IFFT, modifying the vector in place.
    fn ifft_in_place<T: DomainCoeff<F>>(&self, evals: &mut Vec<T>);

    /// Multiply the `i`-th element of `coeffs` with `g^i`.
    fn distribute_powers<T: DomainCoeff<F>>(coeffs: &mut [T], g: F) {
        Self::distribute_powers_and_mul_by_const(coeffs, g, F::one());
    }

    /// Multiply the `i`-th element of `coeffs` with `c*g^i`.
    #[cfg(not(feature = "parallel"))]
    fn distribute_powers_and_mul_by_const<T: DomainCoeff<F>>(coeffs: &mut [T], g: F, c: F) {
        // invariant: pow = c*g^i at the ith iteration of the loop
        let mut pow = c;
        coeffs.iter_mut().for_each(|coeff| {
            *coeff *= pow;
            pow *= &g
        })
    }

    /// Multiply the `i`-th element of `coeffs` with `c*g^i`.
    #[cfg(feature = "parallel")]
    fn distribute_powers_and_mul_by_const<T: DomainCoeff<F>>(coeffs: &mut [T], g: F, c: F) {
        use ark_std::cmp::max;
        let min_parallel_chunk_size = 1024;
        let num_cpus_available = rayon::current_num_threads();
        let num_elem_per_thread = max(coeffs.len() / num_cpus_available, min_parallel_chunk_size);

        ark_std::cfg_chunks_mut!(coeffs, num_elem_per_thread)
            .enumerate()
            .for_each(|(i, chunk)| {
                let offset = c * g.pow([(i * num_elem_per_thread) as u64]);
                let mut pow = offset;
                chunk.iter_mut().for_each(|coeff| {
                    *coeff *= pow;
                    pow *= &g
                })
            });
    }

    /// Evaluate all the lagrange polynomials defined by this domain at the
    /// point `tau`. This is computed in time O(|domain|).
    /// Then given the evaluations of a degree d polynomial P over this domain,
    /// where d < |domain|, `P(tau)` can be computed as
    /// `P(tau) = sum_{i in [|Domain|]} L_{i, Domain}(tau) * P(g^i)`.
    /// `L_{i, Domain}` is the value of the i-th lagrange coefficient
    /// in the returned vector.
    fn evaluate_all_lagrange_coefficients(&self, tau: F) -> Vec<F> {
        // Evaluate all Lagrange polynomials at tau to get the lagrange coefficients.
        // Define the following as
        // - H: The coset we are in, with generator g and offset h
        // - m: The size of the coset H
        // - Z_H: The vanishing polynomial for H. Z_H(x) = prod_{i in m} (x - hg^i) = x^m - h^m
        // - v_i: A sequence of values, where v_0 = 1/(m * h^(m-1)), and v_{i + 1} = g * v_i
        //
        // We then compute L_{i,H}(tau) as `L_{i,H}(tau) = Z_H(tau) * v_i / (tau - h * g^i)`
        //
        // However, if tau in H, both the numerator and denominator equal 0
        // when i corresponds to the value tau equals, and the coefficient is 0
        // everywhere else. We handle this case separately, and we can easily
        // detect by checking if the vanishing poly is 0.
        let size = self.size();
        let z_h_at_tau = self.evaluate_vanishing_polynomial(tau);
        let offset = self.coset_offset();
        let group_gen = self.group_gen();
        if z_h_at_tau.is_zero() {
            // In this case, we know that tau = hg^i, for some value i.
            // Then i-th lagrange coefficient in this case is then simply 1,
            // and all other lagrange coefficients are 0.
            // Thus we find i by brute force.
            let mut u = vec![F::zero(); size];
            let mut omega_i = offset;
            for u_i in u.iter_mut().take(size) {
                if omega_i == tau {
                    *u_i = F::one();
                    break;
                }
                omega_i *= &group_gen;
            }
            u
        } else {
            // In this case we have to compute `Z_H(tau) * v_i / (tau - h g^i)`
            // for i in 0..size
            // We actually compute this by computing (Z_H(tau) * v_i)^{-1} * (tau - h g^i)
            // and then batch inverting to get the correct lagrange coefficients.
            // We let `l_i = (Z_H(tau) * v_i)^-1` and `r_i = tau - h g^i`
            // Notice that since Z_H(tau) is i-independent,
            // and v_i = g * v_{i-1}, it follows that
            // l_i = g^-1 * l_{i-1}
            // TODO: consider caching the computation of l_i to save N multiplications
            use ark_ff::fields::batch_inversion;

            let group_gen_inv = self.group_gen_inv();

            // v_0_inv = m * h^(m-1)
            let v_0_inv = self.size_as_field_element() * offset.pow([size as u64 - 1]);
            let mut l_i = z_h_at_tau.inverse().unwrap() * v_0_inv;
            let mut negative_cur_elem = -offset;
            let mut lagrange_coefficients_inverse = vec![F::zero(); size];
            for coeff in &mut lagrange_coefficients_inverse {
                let r_i = tau + negative_cur_elem;
                *coeff = l_i * r_i;
                // Increment l_i and negative_cur_elem
                l_i *= &group_gen_inv;
                negative_cur_elem *= &group_gen;
            }

            // Invert the lagrange coefficients inverse, to get the actual coefficients,
            // and return these
            batch_inversion(lagrange_coefficients_inverse.as_mut_slice());
            lagrange_coefficients_inverse
        }
    }

    /// Return the sparse vanishing polynomial.
    fn vanishing_polynomial(&self) -> crate::univariate::SparsePolynomial<F> {
        let constant_coeff = self.coset_offset_pow_size();
        let coeffs = vec![(0, -constant_coeff), (self.size(), F::one())];
        crate::univariate::SparsePolynomial::from_coefficients_vec(coeffs)
    }

    /// This evaluates the vanishing polynomial for this domain at tau.
    fn evaluate_vanishing_polynomial(&self, tau: F) -> F {
        // TODO: Consider precomputed exponentiation tables if we need this to be
        // faster.
        tau.pow([self.size() as u64]) - self.coset_offset_pow_size()
    }

    /// Returns the `i`-th element of the domain.
    fn element(&self, i: usize) -> F {
        let mut result = self.group_gen().pow([i as u64]);
        if !self.coset_offset().is_one() {
            result *= self.coset_offset()
        }
        result
    }

    /// Return an iterator over the elements of the domain.
    fn elements(&self) -> Self::Elements;

    /// Given an index which assumes the first elements of this domain are the
    /// elements of another (sub)domain,
    /// this returns the actual index into this domain.
    fn reindex_by_subdomain(&self, other: Self, index: usize) -> usize {
        assert!(self.size() >= other.size());
        // Let this subgroup be G, and the subgroup we're re-indexing by be S.
        // Since its a subgroup, the 0th element of S is at index 0 in G, the first
        // element of S is at index |G|/|S|, the second at 2*|G|/|S|, etc.
        // Thus for an index i that corresponds to S, the index in G is i*|G|/|S|
        let period = self.size() / other.size();
        if index < other.size() {
            index * period
        } else {
            // Let i now be the index of this element in G \ S
            // Let x be the number of elements in G \ S, for every element in S. Then x =
            // (|G|/|S| - 1). At index i in G \ S, the number of elements in S
            // that appear before the index in G to which i corresponds to, is
            // floor(i / x) + 1. The +1 is because index 0 of G is S_0, so the
            // position is offset by at least one. The floor(i / x) term is
            // because after x elements in G \ S, there is one more element from S
            // that will have appeared in G.
            let i = index - other.size();
            let x = period - 1;
            i + (i / x) + 1
        }
    }

    /// Perform O(n) multiplication of two polynomials that are presented by
    /// their evaluations in the domain.
    /// Returns the evaluations of the product over the domain.
    ///
    /// Assumes that the domain is large enough to allow for successful
    /// interpolation after multiplication.
    #[must_use]
    fn mul_polynomials_in_evaluation_domain(&self, self_evals: &[F], other_evals: &[F]) -> Vec<F> {
        assert_eq!(self_evals.len(), other_evals.len());
        let mut result = self_evals.to_vec();

        ark_std::cfg_iter_mut!(result)
            .zip(other_evals)
            .for_each(|(a, b)| *a *= b);

        result
    }
}

/// Types that can be FFT-ed must implement this trait.
pub trait DomainCoeff<F: FftField>:
    Copy
    + Send
    + Sync
    + core::ops::Add<Output = Self>
    + core::ops::Sub<Output = Self>
    + core::ops::AddAssign
    + core::ops::SubAssign
    + ark_ff::Zero
    + core::ops::MulAssign<F>
    + core::fmt::Debug
    + PartialEq
{
}

impl<T, F> DomainCoeff<F> for T
where
    F: FftField,
    T: Copy
        + Send
        + Sync
        + core::ops::Add<Output = Self>
        + core::ops::Sub<Output = Self>
        + core::ops::AddAssign
        + core::ops::SubAssign
        + ark_ff::Zero
        + core::ops::MulAssign<F>
        + core::fmt::Debug
        + PartialEq,
{
}

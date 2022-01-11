use ndarray::{Array1, ArrayBase, ArrayView2, Axis, Data, Ix2};
use ndarray_stats::QuantileExt;
use std::collections::HashMap;

use crate::error::{NaiveBayesError, Result};
use crate::hyperparams::{NbParams, NbValidParams, GaussianNbParams};
use linfa::dataset::{AsTargets, DatasetBase, Labels};
use linfa::{Float, Label};
use crate::base_nb::BaseNb;

// Input and output Gaussian Naive Bayes models for fitting
type GaussianNbIn<F, L> = Option<BaseNb<F, L>>;
type GaussianNbOut<F, L> = Option<BaseNb<F, L>>;

impl<'a, F> GaussianNbParams<F> where F: Float {

 pub fn fit_with<L: Label + 'a,
 D: Data<Elem = F>,
 T: AsTargets<Elem = L> + Labels<Elem = L>>(
        &self,
        model_in: GaussianNbIn<F, L>,
        dataset: &DatasetBase<ArrayBase<D, Ix2>, T>
    ) -> Result<GaussianNbOut<F, L>> {
        let x = dataset.records();
        let y = dataset.try_single_target()?;

        // If the ratio of the variance between dimensions is too small, it will cause
        // numerical errors. We address this by artificially boosting the variance
        // by `epsilon` (a small fraction of the variance of the largest feature)
        let epsilon = self.var_smoothing * *x.var_axis(Axis(0), F::zero()).max()?;

        let mut model = match model_in {
            Some(BaseNb::GaussianNb(mut temp)) => {
                temp.class_info
                    .values_mut()
                    .for_each(|x| x.sigma -= epsilon);
                temp
            }
            None => GaussianNb::<F, L> {
                class_info: HashMap::new(),
            },
            _ => panic!("Wrong model type passed as input - expected Gaussian Naive Bayes")
        };

        let yunique = dataset.labels();

        for class in yunique {
            // We filter for records that correspond to the current class
            let xclass = NbValidParams::filter(x.view(), y.view(), &class);

            // We count the number of occurences of the class
            let nclass = xclass.nrows();

            // We compute the update of the gaussian mean and variance
            let mut class_info = model
                .class_info
                .entry(class)
                .or_insert_with(GaussianClassInfo::default);

            let (theta_new, sigma_new) = Self::update_mean_variance(class_info, xclass.view());

            // We now update the mean, variance and class count
            class_info.theta = theta_new;
            class_info.sigma = sigma_new;
            class_info.class_count += nclass;
        }

        // We add back the epsilon previously subtracted for numerical
        // calculation stability
        model
            .class_info
            .values_mut()
            .for_each(|x| x.sigma += epsilon);

        // We update the priors
        let class_count_sum = model
            .class_info
            .values()
            .map(|x| x.class_count)
            .sum::<usize>();

        for info in model.class_info.values_mut() {
            info.prior = F::cast(info.class_count) / F::cast(class_count_sum);
        }

        Ok(Some(BaseNb::GaussianNb(model)))

    }

    // Compute online update of gaussian mean and variance
    fn update_mean_variance(
        info_old: &GaussianClassInfo<F>,
        x_new: ArrayView2<F>,
    ) -> (Array1<F>, Array1<F>) {
        // Deconstruct old state
        let (count_old, mu_old, var_old) = (info_old.class_count, &info_old.theta, &info_old.sigma);

        // If incoming data is empty no updates required
        if x_new.nrows() == 0 {
            return (mu_old.to_owned(), var_old.to_owned());
        }

        let count_new = x_new.nrows();

        // unwrap is safe because None is returned only when number of records
        // along the specified axis is 0, we return early if we have 0 rows
        let mu_new = x_new.mean_axis(Axis(0)).unwrap();
        let var_new = x_new.var_axis(Axis(0), F::zero());

        // If previous batch was empty, we send the new mean and variance calculated
        if count_old == 0 {
            return (mu_new, var_new);
        }

        let count_total = count_old + count_new;

        // Combine old and new mean, taking into consideration the number
        // of observations
        let mu_new_weighted = &mu_new * F::cast(count_new);
        let mu_old_weighted = mu_old * F::cast(count_old);
        let mu_weighted = (mu_new_weighted + mu_old_weighted).mapv(|x| x / F::cast(count_total));

        // Combine old and new variance, taking into consideration the number
        // of observations. this is achieved by combining the sum of squared
        // differences
        let ssd_old = var_old * F::cast(count_old);
        let ssd_new = var_new * F::cast(count_new);
        let weight = F::cast(count_new * count_old) / F::cast(count_total);
        let ssd_weighted = ssd_old + ssd_new + (mu_old - mu_new).mapv(|x| weight * x.powi(2));
        let var_weighted = ssd_weighted.mapv(|x| x / F::cast(count_total));

        (mu_weighted, var_weighted)
    }

    // Check that the smoothing parameter is non-negative
    pub fn check_ref(&self) -> Result<()> {
        if self.var_smoothing.is_negative() {
            Err(NaiveBayesError::InvalidSmoothing(
                self.var_smoothing.to_f64().unwrap(),
            ))
        } else {
            Ok(())
        }
    }

    
}

/// Fitted Gaussian Naive Bayes classifier
/// 
/// Implements functionality specific to the Gaussian model. Functionality common to
/// all Naive Bayes models is implemented in [`BaseNb`](BaseNb)
#[derive(Debug, Clone)]
pub struct GaussianNb<F, L> {
    class_info: HashMap<L, GaussianClassInfo<F>>,
}

#[derive(Debug, Default, Clone)]
struct GaussianClassInfo<F> {
    class_count: usize,
    prior: F,
    theta: Array1<F>,
    sigma: Array1<F>,
}


impl<F: Float, L: Label> GaussianNb<F, L> where
{
    /// Construct a new set of hyperparameters
    pub fn params() -> NbParams<F, L> {
        NbParams::new().gaussian()
    }

    // Compute unnormalized posterior log probability
    pub fn joint_log_likelihood(&self, x: ArrayView2<F>) -> HashMap<&L, Array1<F>> {
        let mut joint_log_likelihood = HashMap::new();

        for (class, info) in self.class_info.iter() {
            let jointi = info.prior.ln();

            let mut nij = info
                .sigma
                .mapv(|x| F::cast(2. * std::f64::consts::PI) * x)
                .mapv(|x| x.ln())
                .sum();
            nij = F::cast(-0.5) * nij;

            let nij = ((x.to_owned() - &info.theta).mapv(|x| x.powi(2)) / &info.sigma)
                .sum_axis(Axis(1))
                .mapv(|x| x * F::cast(0.5))
                .mapv(|x| nij - x);

            joint_log_likelihood.insert(class, nij + jointi);
        }

        joint_log_likelihood
    }
}


#[cfg(test)]
mod tests {
    use super::{GaussianNb, Result};
    use linfa::{
        traits::{Fit, FitWith, Predict},
        DatasetView,
    };

    use approx::assert_abs_diff_eq;
    use ndarray::{array, Axis};
    use std::collections::HashMap;

    #[test]
    fn test_gaussian_nb() -> Result<()> {
        let x = array![
            [-2., -1.],
            [-1., -1.],
            [-1., -2.],
            [1., 1.],
            [1., 2.],
            [2., 1.]
        ];
        let y = array![1, 1, 1, 2, 2, 2];

        let data = DatasetView::new(x.view(), y.view());
        let fitted_clf = GaussianNb::params().fit(&data)?;
        let pred = fitted_clf.predict(&x);

        assert_abs_diff_eq!(pred, y);

        let jll = fitted_clf.joint_log_likelihood(x.view());
        let mut expected = HashMap::new();
        expected.insert(
            &1usize,
            array![
                -2.276946847943017,
                -1.5269468546930165,
                -2.276946847943017,
                -25.52694663869301,
                -38.27694652394301,
                -38.27694652394301
            ],
        );
        expected.insert(
            &2usize,
            array![
                -38.27694652394301,
                -25.52694663869301,
                -38.27694652394301,
                -1.5269468546930165,
                -2.276946847943017,
                -2.276946847943017
            ],
        );

        assert_eq!(jll, expected);

        Ok(())
    }

    #[test]
    fn test_gnb_fit_with() -> Result<()> {
        let x = array![
            [-2., -1.],
            [-1., -1.],
            [-1., -2.],
            [1., 1.],
            [1., 2.],
            [2., 1.]
        ];
        let y = array![1, 1, 1, 2, 2, 2];

        let clf = GaussianNb::params();

        let model = x
            .axis_chunks_iter(Axis(0), 2)
            .zip(y.axis_chunks_iter(Axis(0), 2))
            .map(|(a, b)| DatasetView::new(a, b))
            .fold(None, |current, d| clf.fit_with(current, &d).unwrap())
            .unwrap();

        let pred = model.predict(&x);

        assert_abs_diff_eq!(pred, y);

        let jll = model.joint_log_likelihood(x.view());

        let mut expected = HashMap::new();
        expected.insert(
            &1usize,
            array![
                -2.276946847943017,
                -1.5269468546930165,
                -2.276946847943017,
                -25.52694663869301,
                -38.27694652394301,
                -38.27694652394301
            ],
        );
        expected.insert(
            &2usize,
            array![
                -38.27694652394301,
                -25.52694663869301,
                -38.27694652394301,
                -1.5269468546930165,
                -2.276946847943017,
                -2.276946847943017
            ],
        );

        for (key, value) in jll.iter() {
            assert_abs_diff_eq!(value, expected.get(key).unwrap(), epsilon = 1e-6);
        }

        Ok(())
    }
}

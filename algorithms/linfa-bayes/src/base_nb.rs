use ndarray::{s, Array1, Array2, ArrayBase, ArrayView1, ArrayView2, Axis, Data, Ix2};
use ndarray_stats::QuantileExt;
use std::collections::HashMap;

use crate::error::{NaiveBayesError, Result};
use linfa::dataset::{AsTargets, DatasetBase, Labels};
use linfa::traits::FitWith;
use linfa::{Float, Label};


pub trait NaiveBayes<'a, F, L, D> where   
    F: Float,
    L: Label + Ord,
    D: Data<Elem = F>,
   // T: AsTargets<Elem = L> + Labels<Elem = L>
     {

    fn joint_log_likelihood(&self, x: ArrayView2<F>) -> HashMap<&L, Array1<F>>;

    fn predict_inplace(&self, x: &ArrayBase<D, Ix2>, y: &mut Array1<L>) {

        assert_eq!(
            x.nrows(),
            y.len(),
            "The number of data points must match the number of output targets."
        );

        let joint_log_likelihood = self.joint_log_likelihood(x.view());

        // We store the classes and likelihood info in an vec and matrix
        // respectively for easier identification of the dominant class for
        // each input
        let nclasses = joint_log_likelihood.keys().len();
        let n = x.nrows();
        let mut classes = Vec::with_capacity(nclasses);
        let mut likelihood = Array2::zeros((nclasses, n));
        joint_log_likelihood
            .iter()
            .enumerate()
            .for_each(|(i, (&key, value))| {
                classes.push(key.clone());
                likelihood.row_mut(i).assign(value);
            });

        // Identify the class with the maximum log likelihood
        *y = likelihood.map_axis(Axis(0), |x| {
            let i = x.argmax().unwrap();
            classes[i].clone()
        });
    }


}



pub trait NaiveBayesValidParams<'a, F, L, D, T>: FitWith<'a, ArrayBase<D, Ix2>, T, NaiveBayesError> where   
    F: Float,
    L: Label + Ord,
    D: Data<Elem = F>,
    T: AsTargets<Elem = L> + Labels<Elem = L>
     {


    fn fit(&self, dataset: &'a DatasetBase<ArrayBase<D, Ix2>, T>, model_none: Self::ObjectIn) -> Result<Self::ObjectOut> {
        // We extract the unique classes in sorted order
        let mut unique_classes = dataset.targets.labels();
        unique_classes.sort_unstable();

        self.fit_with(model_none, dataset)
    }

}




pub fn filter<F: Float, L: Label + Ord>(x: ArrayView2<F>, y: ArrayView1<L>, ycondition: &L) -> Array2<F> {
    // We identify the row numbers corresponding to the class we are interested in
    let index = y
        .into_iter()
        .enumerate()
        .filter_map(|(i, y)| match *ycondition == *y {
            true => Some(i),
            false => None,
        })
        .collect::<Vec<_>>();

    // We subset x to only records corresponding to the class represented in `ycondition`
    let mut xsubset = Array2::zeros((index.len(), x.ncols()));
    index
        .into_iter()
        .enumerate()
        .for_each(|(i, r)| xsubset.row_mut(i).assign(&x.slice(s![r, ..])));

    xsubset
}



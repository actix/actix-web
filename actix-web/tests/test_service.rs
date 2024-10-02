#[cfg(test)]
mod tests {
    use actix_web::services;

    #[test]
    fn test_define_services_macro_with_multiple_arguments() {
        let result = services!(1, 2, 3);
        assert_eq!(result, (1, 2, 3));
    }

    #[test]
    fn test_define_services_macro_with_single_argument() {
        let result = services!(1);
        assert_eq!(result, (1,));
    }

    #[test]
    fn test_define_services_macro_with_no_arguments() {
        let result = services!();
        result
    }

    #[test]
    fn test_define_services_macro_with_trailing_comma() {
        let result = services!(1, 2, 3,);
        assert_eq!(result, (1, 2, 3));
    }

    #[test]
    fn test_define_services_macro_with_comments_in_arguments() {
        let result = services!(
            1, // First comment
            2, // Second comment
            3  // Third comment
        );

        // Assert that comments are ignored and it correctly returns a tuple.
        assert_eq!(result, (1, 2, 3));
    }
}

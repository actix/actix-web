# Changes

## [0.2.1] - 2019-12-22

* Use the same format for file URLs regardless of platforms

## [0.2.0] - 2019-12-20

* Fix BodyEncoding trait import #1220

## [0.2.0-alpha.1] - 2019-12-07

* Migrate to `std::future`

## [0.1.7] - 2019-11-06

* Add an additional `filename*` param in the `Content-Disposition` header of `actix_files::NamedFile` to be more compatible. (#1151)

## [0.1.6] - 2019-10-14

* Add option to redirect to a slash-ended path `Files` #1132

## [0.1.5] - 2019-10-08

* Bump up `mime_guess` crate version to 2.0.1

* Bump up `percent-encoding` crate version to 2.1

* Allow user defined request guards for `Files` #1113

## [0.1.4] - 2019-07-20

* Allow to disable `Content-Disposition` header #686

## [0.1.3] - 2019-06-28

* Do not set `Content-Length` header, let actix-http set it #930

## [0.1.2] - 2019-06-13

* Content-Length is 0 for NamedFile HEAD request #914

* Fix ring dependency from actix-web default features for #741

## [0.1.1] - 2019-06-01

* Static files are incorrectly served as both chunked and with length #812

## [0.1.0] - 2019-05-25

* NamedFile last-modified check always fails due to nano-seconds
  in file modified date #820

## [0.1.0-beta.4] - 2019-05-12

* Update actix-web to beta.4

## [0.1.0-beta.1] - 2019-04-20

* Update actix-web to beta.1

## [0.1.0-alpha.6] - 2019-04-14

* Update actix-web to alpha6

## [0.1.0-alpha.4] - 2019-04-08

* Update actix-web to alpha4

## [0.1.0-alpha.2] - 2019-04-02

* Add default handler support

## [0.1.0-alpha.1] - 2019-03-28

* Initial impl

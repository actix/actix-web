# Changes

## [0.1.5] - unreleased

* Bump up `mime_guess` crate version to 2.0.1

* Bump up `percent-encoding` crate version to 2.1

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

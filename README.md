# Sentinel (monitoring)

## About

Simple monitoring software, which can monitor various resources (currently only HTTP) and notify, if something go wrong (currently, only via SMTP).

## Installation

For now only available option is to clone this repo and build it yourself with `cargo`.

## Usage

Simple run executable. Configuration is written in YAML. Default path to configuration is `./config.yml`. Custom path can be provided via `CONFIG` environment variable.

## Configuration example

``` yaml
resources:
  - name: example-dot-com
    type: http
    interval: 60000
    notifiers:
      - smtp
    config:
      url: "http://example.com"
      codes:
        Success:
          - 200
  - name: example-dot-com-404
    type: http
    interval: 60000
    notifiers:
      - smtp
    config:
      url: "http://example.com/404"
      codes:
        Success:
          - 404
notifiers:
  - name: smtp
    type: smtp
    config:
      host: "<your_smtp_host>"
      login: "<your_smtp_login>"
      pwd: "<your_smtp_password>"
      ident: "Sentinel"
      recipients:
        - address: "<where_to_send_notifications>"
        - address: "<where_to_send_notifications>"
```


#!/bin/sh
# Generates high-volume Spring Boot Logstash-encoder JSON logs with realistic
# stacktraces, large payloads, and varied log levels / logger names.
# No sleep — runs as fast as the shell allows.

# Stacktraces stored as JSON-safe single-line strings (literal \n, not newlines)
STACK_NPE='java.lang.NullPointerException: Cannot invoke \"String.length()\" because \"str\" is null\n\tat com.example.service.OrderService.validatePayload(OrderService.java:214)\n\tat com.example.service.OrderService.processOrder(OrderService.java:142)\n\tat com.example.service.OrderService$$FastClassBySpringCGLIB$$1.invoke(<generated>)\n\tat org.springframework.aop.framework.ReflectiveMethodInvocation.proceed(ReflectiveMethodInvocation.java:186)\n\tat org.springframework.transaction.interceptor.TransactionInterceptor.invoke(TransactionInterceptor.java:123)\n\tat org.springframework.aop.framework.ReflectiveMethodInvocation.proceed(ReflectiveMethodInvocation.java:186)\n\tat com.example.controller.OrderController.createOrder(OrderController.java:67)\n\tat sun.reflect.NativeMethodAccessorImpl.invoke0(Native Method)\n\tat sun.reflect.NativeMethodAccessorImpl.invoke(NativeMethodAccessorImpl.java:62)\n\tat org.springframework.web.servlet.FrameworkServlet.service(FrameworkServlet.java:897)\n\tat javax.servlet.http.HttpServlet.service(HttpServlet.java:741)\n\t... 38 more'

STACK_TIMEOUT='java.net.SocketTimeoutException: Read timed out\n\tat java.net.SocketInputStream.socketRead0(Native Method)\n\tat java.net.SocketInputStream.socketRead(SocketInputStream.java:116)\n\tat java.net.SocketInputStream.read(SocketInputStream.java:171)\n\tat com.example.client.PaymentGatewayClient.charge(PaymentGatewayClient.java:88)\n\tat com.example.service.PaymentService.processPayment(PaymentService.java:55)\n\tat com.example.service.OrderService.fulfil(OrderService.java:301)\n\tat com.example.service.OrderService$$FastClassBySpringCGLIB$$1.invoke(<generated>)\n\tat org.springframework.aop.framework.ReflectiveMethodInvocation.proceed(ReflectiveMethodInvocation.java:186)\n\tat org.springframework.transaction.interceptor.TransactionInterceptor.invoke(TransactionInterceptor.java:123)\n\tat com.example.controller.CheckoutController.checkout(CheckoutController.java:44)\n\t... 52 more'

STACK_DB='org.springframework.dao.DataAccessResourceFailureException: Could not open JPA EntityManager for transaction\n\tat org.springframework.orm.jpa.JpaTransactionManager.doBegin(JpaTransactionManager.java:450)\n\tat org.springframework.transaction.support.AbstractPlatformTransactionManager.startTransaction(AbstractPlatformTransactionManager.java:400)\nCaused by: org.hibernate.exception.JDBCConnectionException: Unable to acquire JDBC Connection\n\tat org.hibernate.exception.internal.SQLExceptionTypeDelegate.convert(SQLExceptionTypeDelegate.java:48)\n\tat org.hibernate.engine.jdbc.spi.SqlExceptionHelper.convert(SqlExceptionHelper.java:113)\nCaused by: java.sql.SQLTransientConnectionException: HikariPool-1 - Connection is not available, request timed out after 30000ms\n\tat com.zaxxer.hikari.pool.HikariPool.getConnection(HikariPool.java:227)\n\tat com.zaxxer.hikari.HikariDataSource.getConnection(HikariDataSource.java:100)\n\t... 89 more'

STACK_OOM='java.lang.OutOfMemoryError: Java heap space\n\tat java.util.Arrays.copyOf(Arrays.java:3210)\n\tat java.util.Arrays.copyOf(Arrays.java:3181)\n\tat java.util.ArrayList.grow(ArrayList.java:265)\n\tat java.util.ArrayList.ensureExplicitCapacity(ArrayList.java:239)\n\tat com.example.service.ReportService.buildReport(ReportService.java:512)\n\tat com.example.service.ReportService.generate(ReportService.java:78)\n\tat com.example.scheduler.ReportScheduler.run(ReportScheduler.java:34)\n\tat org.springframework.scheduling.support.DelegatingErrorHandlingRunnable.run(DelegatingErrorHandlingRunnable.java:54)\n\t... 12 more'

i=0
while true; do
  mod7=$((i % 7))
  mod13=$((i % 13))
  mod50=$((i % 50))
  mod3=$((i % 3))
  mod6=$((i % 6))
  mod4=$((i % 4))

  # Level
  if [ "$mod13" -eq 0 ]; then
    level=ERROR; level_value=40000
  elif [ "$mod7" -eq 0 ]; then
    level=WARN; level_value=30000
  elif [ "$mod50" -eq 0 ]; then
    level=DEBUG; level_value=10000
  else
    level=INFO; level_value=20000
  fi

  # Logger
  if [ "$mod6" -eq 0 ]; then logger="com.example.service.OrderService"
  elif [ "$mod6" -eq 1 ]; then logger="com.example.service.PaymentService"
  elif [ "$mod6" -eq 2 ]; then logger="com.example.controller.CheckoutController"
  elif [ "$mod6" -eq 3 ]; then logger="com.example.repository.ProductRepository"
  elif [ "$mod6" -eq 4 ]; then logger="org.springframework.web.servlet.DispatcherServlet"
  else logger="org.hibernate.SQL"
  fi

  # Thread
  if [ "$mod4" -eq 0 ]; then thread="http-nio-8080-exec-$((1 + i % 8))"
  elif [ "$mod4" -eq 1 ]; then thread="task-scheduler-$((1 + i % 3))"
  elif [ "$mod4" -eq 2 ]; then thread="async-worker-$((1 + i % 4))"
  else thread="kafka-consumer-$((1 + i % 2))"
  fi

  # Message and extra fields — bigger payloads on select iterations
  order_id="ORD-$((10000 + i % 90000))"
  user_id="usr-$((1000 + i % 9000))"
  latency=$((12 + i % 988))
  span_id=$(printf '%016x' "$i")
  trace_id=$(printf '%032x' "$((i * 31337))")

  if [ "$level" = "ERROR" ]; then
    # Pick a stacktrace
    stack_mod=$((i % 4))
    if [ "$stack_mod" -eq 0 ]; then stack="$STACK_NPE"
    elif [ "$stack_mod" -eq 1 ]; then stack="$STACK_TIMEOUT"
    elif [ "$stack_mod" -eq 2 ]; then stack="$STACK_DB"
    else stack="$STACK_OOM"
    fi
    printf '{"@timestamp":"2026-05-06T12:%02d:%02dZ","@version":"1","message":"Unhandled exception processing request for order %s","logger_name":"%s","thread_name":"%s","level":"%s","level_value":%s,"order_id":"%s","user_id":"%s","latency_ms":%s,"trace_id":"%s","span_id":"%s","stack_trace":"%s"}\n' \
      $((i % 60)) $((i * 7 % 60)) "$order_id" "$logger" "$thread" "$level" "$level_value" "$order_id" "$user_id" "$latency" "$trace_id" "$span_id" "$stack"
  elif [ "$level" = "WARN" ]; then
    printf '{"@timestamp":"2026-05-06T12:%02d:%02dZ","@version":"1","message":"Slow database query detected for order %s — %dms exceeds threshold","logger_name":"%s","thread_name":"%s","level":"%s","level_value":%s,"order_id":"%s","user_id":"%s","latency_ms":%s,"threshold_ms":500,"trace_id":"%s","span_id":"%s"}\n' \
      $((i % 60)) $((i * 7 % 60)) "$order_id" "$latency" "$logger" "$thread" "$level" "$level_value" "$order_id" "$user_id" "$latency" "$trace_id" "$span_id"
  elif [ "$level" = "DEBUG" ]; then
    # Large debug payload with request/response bodies
    printf '{"@timestamp":"2026-05-06T12:%02d:%02dZ","@version":"1","message":"Outbound HTTP request","logger_name":"%s","thread_name":"%s","level":"%s","level_value":%s,"order_id":"%s","user_id":"%s","latency_ms":%s,"trace_id":"%s","span_id":"%s","request_body":"{\"orderId\":\"%s\",\"userId\":\"%s\",\"items\":[{\"sku\":\"SKU-%04d\",\"qty\":%d,\"price\":%.2f},{\"sku\":\"SKU-%04d\",\"qty\":%d,\"price\":%.2f}],\"shippingAddress\":{\"street\":\"123 Main St\",\"city\":\"Springfield\",\"zip\":\"12345\"}}","response_body":"{\"status\":\"ACCEPTED\",\"paymentRef\":\"PAY-%08d\",\"estimatedDelivery\":\"2026-05-10\"}"}\n' \
      $((i % 60)) $((i * 7 % 60)) "$logger" "$thread" "$level" "$level_value" "$order_id" "$user_id" "$latency" "$trace_id" "$span_id" \
      "$order_id" "$user_id" $((1000 + i % 9000)) $((1 + i % 5)) $(printf '%d.%02d' $((10 + i % 90)) $((i % 100))) \
      $((2000 + i % 8000)) $((1 + i % 3)) $(printf '%d.%02d' $((5 + i % 50)) $((i % 100))) \
      $((100000 + i))
  else
    printf '{"@timestamp":"2026-05-06T12:%02d:%02dZ","@version":"1","message":"Processed order %s successfully","logger_name":"%s","thread_name":"%s","level":"%s","level_value":%s,"order_id":"%s","user_id":"%s","latency_ms":%s,"trace_id":"%s","span_id":"%s"}\n' \
      $((i % 60)) $((i * 7 % 60)) "$order_id" "$logger" "$thread" "$level" "$level_value" "$order_id" "$user_id" "$latency" "$trace_id" "$span_id"
  fi

  i=$((i + 1))
done

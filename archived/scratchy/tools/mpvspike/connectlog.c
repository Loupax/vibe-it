/* LD_PRELOAD shim: log every connect() with the calling thread's name.
   strace is not installed; this answers "does libmpv connect()?" for QM1b.
   gcc -shared -fPIC -o libconnectlog.so connectlog.c -ldl */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <sys/prctl.h>
int connect(int fd, const struct sockaddr *a, socklen_t l) {
  static int (*real)(int, const struct sockaddr *, socklen_t);
  if (!real) real = dlsym(RTLD_NEXT, "connect");
  char name[32] = "?"; prctl(PR_GET_NAME, name, 0, 0, 0);
  if (a->sa_family == AF_INET) {
    const struct sockaddr_in *i = (const void *)a; char b[64];
    inet_ntop(AF_INET, &i->sin_addr, b, sizeof b);
    fprintf(stderr, "CONNECT thread=%s inet %s:%d\n", name, b, ntohs(i->sin_port));
  } else if (a->sa_family == AF_INET6) {
    fprintf(stderr, "CONNECT thread=%s inet6\n", name);
  } else if (a->sa_family == AF_UNIX) {
    const struct sockaddr_un *u = (const void *)a;
    fprintf(stderr, "CONNECT thread=%s unix %s\n", name, u->sun_path[0] ? u->sun_path : "@abstract");
  } else fprintf(stderr, "CONNECT thread=%s family=%d\n", name, a->sa_family);
  return real(fd, a, l);
}

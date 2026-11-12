/* Test utility only. It connects exclusively to our isolated compositor socket.
 * Generate fake-input-client-protocol.h and fake-input-protocol.c with wayland-scanner
 * from the installed plasma-wayland-protocols/fake-input.xml, then link wayland-client.
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <wayland-client.h>
#include "fake-input-client-protocol.h"

static struct org_kde_kwin_fake_input *input;
static void global(void *data,struct wl_registry *registry,uint32_t name,const char *interface,uint32_t version){
    (void)data;
    if(strcmp(interface,"org_kde_kwin_fake_input")==0 && version>=4)
        input=wl_registry_bind(registry,name,&org_kde_kwin_fake_input_interface,version<6?version:6);
}
static void removed(void *data,struct wl_registry *registry,uint32_t name){(void)data;(void)registry;(void)name;}
static const struct wl_registry_listener listener={global,removed};
int main(void){
    /* Never use the user's normal WAYLAND_DISPLAY for synthetic input. */
    struct wl_display *display=wl_display_connect("tide-qa-test");
    if(!display){fputs("Isolated Tide QA compositor is unavailable\n",stderr);return 1;}
    struct wl_registry *registry=wl_display_get_registry(display);
    wl_registry_add_listener(registry,&listener,NULL);wl_display_roundtrip(display);
    if(!input){fputs("QA compositor has no fake-input interface\n",stderr);return 1;}
    org_kde_kwin_fake_input_authenticate(input,"Tide isolated GUI tests","Exercise the terminal inside the dedicated QA compositor");
    wl_display_roundtrip(display);
    char line[128],kind[16];int a,b;
    while(fgets(line,sizeof line,stdin)){
        if(sscanf(line,"%15s %d %d",kind,&a,&b)!=3)return 2;
        if(strcmp(kind,"key")==0){if(a<8||a>255)return 2;org_kde_kwin_fake_input_keyboard_key(input,a-8,b);}
        else if(strcmp(kind,"motion")==0)org_kde_kwin_fake_input_pointer_motion_absolute(input,wl_fixed_from_int(a),wl_fixed_from_int(b));
        else if(strcmp(kind,"button")==0){uint32_t code=a==1?0x110:a==2?0x112:0x111;org_kde_kwin_fake_input_button(input,code,b);}
        else return 2;
        if(wl_display_roundtrip(display)<0)return 3;
        puts("OK");fflush(stdout);
    }
    wl_display_disconnect(display);return 0;
}
